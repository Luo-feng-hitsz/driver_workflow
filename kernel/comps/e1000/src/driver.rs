// SPDX-License-Identifier: MPL-2.0

//! Top-level e1000 driver: PCI probe, adapter struct, open/close, reset,
//! watchdog, link state management, and AnyNetworkDevice implementation.
//! Translated from e1000_main.c and e1000.h.

use alloc::{string::ToString, sync::Arc};
use core::fmt::Debug;

use aster_bigtcp::device::{Checksum, DeviceCapabilities, Medium};
use aster_network::{AnyNetworkDevice, EthernetAddr, NetError, RxBuffer};
use aster_pci::{
    PciDeviceId,
    bus::{PciDevice, PciDriver},
    cfg_space::{Bar, PciCommonCfgOffset},
    common_device::PciCommonDevice,
};
use ostd::{
    arch::trap::TrapFrame,
    bus::BusProbeError,
    debug, error, info, warn,
    irq::IrqLine,
    mm::dma::FromDevice,
    sync::SpinLock,
};
use spin::Once;

use crate::hw::E1000Hw;
use crate::intr;
use crate::regs::E1000Regs;
use crate::rx::{self, RxRing, RX_BUFFER_SIZE};
use crate::tx::{self, TxRing};

// ============================================================================
// Constants
// ============================================================================

/// Intel PCI vendor ID.
const INTEL_VENDOR_ID: u16 = 0x8086;

/// Device IDs supported by this driver.
const SUPPORTED_DEVICE_IDS: &[u16] = &[
    0x100E, 0x1015, 0x1016, 0x1017, 0x101E, 0x100F, 0x1011, 0x1010, 0x1012,
    0x1013, 0x1014, 0x1018, 0x1076, 0x1077, 0x1078, 0x1079, 0x107A, 0x107B,
    0x107C, 0x108A, 0x1099, 0x10B5, 0x1000, 0x1001, 0x1004, 0x1008, 0x1009,
    0x100C, 0x100D, 0x1019, 0x101A, 0x101D, 0x1026, 0x1027, 0x1028, 0x1075,
    0x2E6E,
];

/// Network device name.
pub const DEVICE_NAME: &str = "e1000-net";

/// RX buffer DMA pool for the e1000 driver.
static RX_POOL: Once<Arc<aster_network::dma_pool::DmaPool<FromDevice>>> = Once::new();
static IRQ_LINE: Once<ostd::arch::irq::MappedIrqLine> = Once::new();
static IRQ_REGS: Once<crate::regs::E1000Regs> = Once::new();

// ============================================================================
// PCI Driver
// ============================================================================

/// The PCI driver instance for e1000.
#[derive(Debug)]
pub struct E1000PciDriver;

impl PciDriver for E1000PciDriver {
    fn probe(
        &self,
        device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (BusProbeError, PciCommonDevice)> {
        // Check vendor
        if device.device_id().vendor_id != INTEL_VENDOR_ID {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        // Check device ID
        let dev_id = device.device_id().device_id;
        if !SUPPORTED_DEVICE_IDS.contains(&dev_id) {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        // Attempt to initialize the device
        match E1000Device::init(device) {
            Ok(pci_device) => {
                info!("found Intel e1000 NIC: {:04x}:{:04x}", INTEL_VENDOR_ID, dev_id);
                Ok(pci_device)
            }
            Err((err_msg, device)) => {
                error!("e1000: probe failed: {}", err_msg);
                Err((BusProbeError::ConfigurationSpaceError, device))
            }
        }
    }
}

// ============================================================================
// PCI Device wrapper
// ============================================================================

/// Wrapper satisfying the PciDevice trait for the claimed e1000 device.
#[derive(Debug)]
struct E1000PciDeviceWrapper {
    device_id: PciDeviceId,
}

impl PciDevice for E1000PciDeviceWrapper {
    fn device_id(&self) -> PciDeviceId {
        self.device_id
    }
}

// ============================================================================
// E1000 Network Device (Adapter)
// ============================================================================

/// The e1000 adapter / network device.
pub struct E1000Device {
    /// Hardware abstraction (registers, MAC, PHY state).
    hw: E1000Hw,
    /// TX ring.
    tx_ring: TxRing,
    /// RX ring.
    rx_ring: RxRing,
    /// MAC address.
    mac_addr: EthernetAddr,
    /// Device capabilities for smoltcp/bigtcp.
    caps: DeviceCapabilities,
    /// Link is up.
    link_up: bool,
    /// Link speed in Mbps.
    link_speed: u16,
    /// Link duplex (1=half, 2=full).
    link_duplex: u16,
}

impl E1000Device {
    /// Initializes the e1000 device from a PCI common device.
    fn init(
        mut device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (&'static str, PciCommonDevice)> {
        let device_id = *device.device_id();

        // Get BAR 0 (MMIO register space) and acquire IoMem
        let io_mem = {
            let bar = device.bar_manager_mut().bar_mut(0);
            let bar = match bar {
                Some(bar) => bar,
                None => return Err(("BAR 0 not found", device)),
            };

            match bar {
                Bar::Memory(mem_bar) => {
                    match mem_bar.acquire() {
                        Ok(io_mem) => io_mem.clone(),
                        Err(_) => return Err(("Failed to acquire IoMem from BAR 0", device)),
                    }
                }
                _ => return Err(("BAR 0 is not a memory BAR", device)),
            }
        };

        // Initialize the RX DMA pool (once)
        RX_POOL.call_once(|| {
            aster_network::dma_pool::DmaPool::new(
                RX_BUFFER_SIZE, // segment size
                32,             // initial pages
                64,             // high watermark
                false,          // not cache coherent
            )
        });

        let regs = E1000Regs::new(io_mem);
        IRQ_REGS.call_once(|| regs.clone());
        let mut hw = E1000Hw::new(regs);

        // Reset the hardware
        hw.reset_hw();

        // Initialize EEPROM and read MAC
        if let Err(e) = crate::eeprom::validate_eeprom_checksum(&hw) {
            warn!("e1000: EEPROM checksum validation failed: {}", e);
            // Continue anyway, some virtual NICs don't have valid checksums
        }

        if let Err(e) = hw.read_mac_addr() {
            // Try a fallback: generate a locally-administered MAC
            warn!("e1000: Failed to read MAC from EEPROM: {}, using fallback", e);
            hw.mac_addr = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
            hw.perm_mac_addr = hw.mac_addr;
        }

        let mac_addr = EthernetAddr(hw.mac_addr);
        info!("MAC address: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            hw.mac_addr[0], hw.mac_addr[1], hw.mac_addr[2],
            hw.mac_addr[3], hw.mac_addr[4], hw.mac_addr[5]);

        // Initialize hardware
        if let Err(e) = hw.init_hw() {
            return Err((e, device));
        }

        // Create TX and RX rings
        let tx_ring = match TxRing::new() {
            Ok(r) => r,
            Err(e) => return Err((e, device)),
        };
        let rx_ring = match RxRing::new(RX_POOL.get().unwrap().clone()) {
            Ok(r) => r,
            Err(e) => return Err((e, device)),
        };

        // Configure TX and RX hardware
        tx::configure_tx(&hw, &tx_ring);
        rx::configure_rx(&hw, &rx_ring);

        // Set up device capabilities
        let mut caps = DeviceCapabilities::default();
        caps.max_burst_size = None;
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1500;
        caps.checksum.tcp = Checksum::Both;
        caps.checksum.udp = Checksum::Both;
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.icmpv4 = Checksum::Both;

        let mut adapter = E1000Device {
            hw,
            tx_ring,
            rx_ring,
            mac_addr,
            caps,
            link_up: false,
            link_speed: 0,
            link_duplex: 0,
        };

        // Allocate RX buffers
        if let Err(e) = adapter.rx_ring.alloc_rx_buffers(&adapter.hw) {
            error!("e1000: Failed to allocate RX buffers: {}", e);
        }

        // Enable interrupts
        intr::irq_enable(&adapter.hw);

        // Check initial link status
        let (up, speed, duplex) = adapter.hw.check_for_link();
        adapter.link_up = up;
        if let Some(s) = speed {
            adapter.link_speed = match s {
                crate::hw::LinkSpeed::Speed10 => 10,
                crate::hw::LinkSpeed::Speed100 => 100,
                crate::hw::LinkSpeed::Speed1000 => 1000,
            };
        }
        if let Some(d) = duplex {
            adapter.link_duplex = match d {
                crate::hw::Duplex::Half => 1,
                crate::hw::Duplex::Full => 2,
            };
        }

        // Register the network device with aster-network
        let device_ref = Arc::new(SpinLock::new(adapter));
        aster_network::register_device(DEVICE_NAME.to_string(), device_ref);

        // Set up interrupt handler via legacy PCI INTx
        let gsi = device.location().read8(PciCommonCfgOffset::InterruptLine as u16) as u32;
        if gsi > 0 {
            if let Ok(mut irq_line) = IrqLine::alloc() {
                irq_line.on_active(handle_interrupt);
                match ostd::arch::irq::IRQ_CHIP.get().unwrap().map_gsi_pin_to(irq_line, gsi) {
                    Ok(mapped_irq) => {
                        // Keep the mapped IRQ alive for the lifetime of the driver
                        IRQ_LINE.call_once(|| mapped_irq);
                        info!("registered IRQ for GSI {}", gsi);
                    }
                    Err(e) => {
                        warn!("failed to map GSI {} to IRQ: {:?}", gsi, e);
                    }
                }
            }
        }

        debug!("e1000: Device initialized, MAC = {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac_addr.0[0], mac_addr.0[1], mac_addr.0[2],
            mac_addr.0[3], mac_addr.0[4], mac_addr.0[5]);

        Ok(Arc::new(E1000PciDeviceWrapper { device_id }))
    }
}

// ============================================================================
// AnyNetworkDevice implementation
// ============================================================================

impl AnyNetworkDevice for E1000Device {
    fn mac_addr(&self) -> EthernetAddr {
        self.mac_addr
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.caps.clone()
    }

    fn can_receive(&self) -> bool {
        self.rx_ring.can_receive()
    }

    fn can_send(&self) -> bool {
        self.tx_ring.can_send()
    }

    fn receive(&mut self) -> Result<RxBuffer, NetError> {
        match self.rx_ring.clean_rx_irq(&self.hw) {
            Some(buf) => Ok(buf),
            None => Err(NetError::NotReady),
        }
    }

    fn send(&mut self, packet: &[u8]) -> Result<(), NetError> {
        if !self.can_send() {
            return Err(NetError::Busy);
        }
        self.tx_ring
            .xmit_frame(&self.hw, packet)
            .map_err(|_| NetError::NoMemory)
    }

    fn free_processed_tx_buffers(&mut self) {
        self.tx_ring.clean_tx_irq();
    }

    fn notify_poll_end(&mut self) {
        // Re-enable interrupts after polling
        intr::irq_enable(&self.hw);
    }
}

impl Debug for E1000Device {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("E1000Device")
            .field("mac_addr", &self.mac_addr)
            .field("link_up", &self.link_up)
            .field("link_speed", &self.link_speed)
            .finish()
    }
}

// ============================================================================
// Interrupt Handler
// ============================================================================

/// Top-level interrupt handler for the e1000.
/// Called from the IRQ framework.
pub fn handle_interrupt(_frame: &TrapFrame) {
    // Read ICR to acknowledge and clear the interrupt (read-to-clear register).
    // Without this, INTx stays asserted and the interrupt gets masked permanently.
    if let Some(regs) = IRQ_REGS.get() {
        let icr = regs.read(crate::regs::ICR);
        if icr == 0 {
            return; // Not our interrupt
        }
    }
    aster_network::raise_receive_softirq();
    aster_network::raise_send_softirq();
}
