// SPDX-License-Identifier: MPL-2.0

//! Top-level e1000e driver: PCI probe/remove, open/close, reset, interrupt
//! setup (INTx for 82574), link state management, and AnyNetworkDevice trait
//! implementation.

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

use crate::desc;
use crate::regs::*;
use crate::rx::{RxCoalesceConfig, RxRing};
use crate::tx::{self, TxRing};

/// Device name used to register with `aster_network`.
pub const DEVICE_NAME: &str = "e1000e-net";

/// Intel PCI vendor ID.
const INTEL_VENDOR_ID: u16 = 0x8086;

/// PCI device ID for the 82574L.
const E1000E_DEV_ID_82574L: u16 = 0x10D3;

/// Supported device IDs for this driver (82574 family).
const SUPPORTED_DEVICE_IDS: &[u16] = &[
    E1000E_DEV_ID_82574L,
    0x10F6, // 82574L variant
];

/// RX buffer size (2048 bytes, standard Ethernet frames).
const RX_BUFFER_SIZE: usize = 2048;

/// Number of RX descriptors.
const RX_DESC_COUNT: u16 = 256;

/// Static storage for IRQ resources that must live for the driver lifetime.
static RX_POOL: Once<Arc<aster_network::dma_pool::DmaPool<FromDevice>>> = Once::new();
static IRQ_LINE: Once<ostd::arch::irq::MappedIrqLine> = Once::new();
static IRQ_REGS: Once<E1000eRegs> = Once::new();

/// The PCI driver instance for e1000e.
#[derive(Debug)]
pub(crate) struct E1000ePciDriver;

/// Wrapper satisfying the `PciDevice` trait for the claimed e1000e device.
#[derive(Debug)]
struct E1000ePciDeviceWrapper {
    device_id: PciDeviceId,
}

impl PciDevice for E1000ePciDeviceWrapper {
    fn device_id(&self) -> PciDeviceId {
        self.device_id
    }
}

impl PciDriver for E1000ePciDriver {
    fn probe(
        &self,
        device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (BusProbeError, PciCommonDevice)> {
        // Check vendor ID.
        if device.device_id().vendor_id != INTEL_VENDOR_ID {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        // Check device ID.
        let dev_id = device.device_id().device_id;
        if !SUPPORTED_DEVICE_IDS.contains(&dev_id) {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        // Attempt full initialization.
        match E1000eDevice::init(device) {
            Ok(pci_device) => {
                info!("found Intel e1000e NIC: {:04x}:{:04x}", INTEL_VENDOR_ID, dev_id);
                Ok(pci_device)
            }
            Err((err_msg, device)) => {
                error!("e1000e: probe failed: {}", err_msg);
                Err((BusProbeError::ConfigurationSpaceError, device))
            }
        }
    }
}

// ============================================================================
// E1000e Network Device (Adapter)
// ============================================================================

/// The e1000e adapter / network device.
pub struct E1000eDevice {
    /// MMIO register accessor.
    regs: E1000eRegs,
    /// TX ring.
    tx_ring: TxRing,
    /// RX ring.
    rx_ring: RxRing,
    /// MAC address.
    mac_addr: EthernetAddr,
    /// Device capabilities for smoltcp/bigtcp.
    caps: DeviceCapabilities,
}

impl E1000eDevice {
    /// Initializes the e1000e device from a PCI common device.
    fn init(
        mut device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (&'static str, PciCommonDevice)> {
        let device_id = *device.device_id();

        // ---- BAR 0 (MMIO register space) ----
        let io_mem = {
            let bar = device.bar_manager_mut().bar_mut(0);
            let bar = match bar {
                Some(bar) => bar,
                None => return Err(("BAR 0 not found", device)),
            };
            match bar {
                Bar::Memory(mem_bar) => match mem_bar.acquire() {
                    Ok(io_mem) => io_mem.clone(),
                    Err(_) => return Err(("Failed to acquire IoMem from BAR 0", device)),
                },
                _ => return Err(("BAR 0 is not a memory BAR", device)),
            }
        };

        let regs = E1000eRegs::new(io_mem);

        // ---- RX DMA pool (once) ----
        RX_POOL.call_once(|| {
            aster_network::dma_pool::DmaPool::new(
                RX_BUFFER_SIZE,
                32,
                64,
                false,
            )
        });

        // ---- Hardware reset ----
        hw_reset(&regs);

        // ---- Read MAC address from EEPROM via EERD ----
        let mac = match read_mac_from_eerd(&regs) {
            Ok(m) => m,
            Err(_) => {
                warn!("e1000e: failed to read MAC from EEPROM, using fallback");
                [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]
            }
        };

        let mac_addr = EthernetAddr(mac);
        info!(
            "e1000e: MAC address: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );

        // ---- Program Receive Address (RAR 0) ----
        rar_set(&regs, &mac, 0);

        // ---- Zero multicast table ----
        for i in 0..NUM_MTA_REGISTERS {
            regs.write(MTA + (i * 4), 0);
        }

        // ---- Set up link: force link up, autoneg, copper ----
        {
            let ctrl = regs.read(CTRL);
            regs.write(CTRL, ctrl | CTRL_SLU | CTRL_ASDE);
        }

        // ---- Create TX ring and configure hardware ----
        let tx_ring = match TxRing::new() {
            Ok(r) => r,
            Err(e) => return Err((e, device)),
        };
        tx::configure_tx(&regs, &tx_ring);

        // ---- Create RX ring ----
        let rx_pool = RX_POOL.get().unwrap().clone();
        let desc_ring = match desc::alloc_rx_ring(RX_DESC_COUNT, &rx_pool) {
            Ok(r) => r,
            Err(_) => return Err(("Failed to allocate RX descriptor ring", device)),
        };
        let rx_ring = RxRing::new(desc_ring, rx_pool, true, true, true);

        // ---- Configure RX hardware ----
        // First set up RCTL
        rx_ring.setup_rctl(regs.io_mem(), 0);
        // Then configure RX ring registers
        rx_ring.configure_rx(regs.io_mem(), &RxCoalesceConfig::default());

        // ---- Set RDT to make all descriptors available to hardware ----
        // RDT should point to the last valid descriptor (count - 1)
        regs.write(RDT, (RX_DESC_COUNT - 1) as u32);

        // ---- Set up device capabilities ----
        let mut caps = DeviceCapabilities::default();
        caps.max_burst_size = None;
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1500;
        caps.checksum.tcp = Checksum::Both;
        caps.checksum.udp = Checksum::Both;
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.icmpv4 = Checksum::Both;

        let adapter = E1000eDevice {
            regs: regs.clone(),
            tx_ring,
            rx_ring,
            mac_addr,
            caps,
        };

        // ---- Enable interrupts ----
        regs.write(IMS, IMS_ENABLE_MASK);
        let _ = regs.read(STATUS); // flush

        // ---- Register the network device ----
        let device_ref = Arc::new(SpinLock::new(adapter));
        aster_network::register_device(DEVICE_NAME.to_string(), device_ref);

        // ---- Save regs for interrupt handler ----
        IRQ_REGS.call_once(|| regs);

        // ---- Set up interrupt handler via legacy PCI INTx ----
        let gsi = device.location().read8(PciCommonCfgOffset::InterruptLine as u16) as u32;
        if gsi > 0 {
            if let Ok(mut irq_line) = IrqLine::alloc() {
                irq_line.on_active(handle_interrupt);
                match ostd::arch::irq::IRQ_CHIP
                    .get()
                    .unwrap()
                    .map_gsi_pin_to(irq_line, gsi)
                {
                    Ok(mapped_irq) => {
                        IRQ_LINE.call_once(|| mapped_irq);
                        info!("e1000e: registered IRQ for GSI {}", gsi);
                    }
                    Err(e) => {
                        warn!("e1000e: failed to map GSI {} to IRQ: {:?}", gsi, e);
                    }
                }
            }
        }

        debug!(
            "e1000e: device initialized, MAC = {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );

        Ok(Arc::new(E1000ePciDeviceWrapper { device_id }))
    }
}

// ============================================================================
// AnyNetworkDevice implementation
// ============================================================================

impl AnyNetworkDevice for E1000eDevice {
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
        let received = self.rx_ring.clean_rx_irq(self.regs.io_mem(), 1);
        if let Some(buf) = received.into_iter().next() {
            Ok(buf)
        } else {
            Err(NetError::NotReady)
        }
    }

    fn send(&mut self, packet: &[u8]) -> Result<(), NetError> {
        if !self.can_send() {
            return Err(NetError::Busy);
        }
        self.tx_ring
            .xmit_frame(&self.regs, packet)
            .map_err(|_| NetError::NoMemory)
    }

    fn free_processed_tx_buffers(&mut self) {
        self.tx_ring.clean_tx_irq();
    }

    fn notify_poll_end(&mut self) {
        // Re-enable interrupts after polling
        self.regs.write(IMS, IMS_ENABLE_MASK);
        let _ = self.regs.read(STATUS); // flush
    }
}

impl Debug for E1000eDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("E1000eDevice")
            .field("mac_addr", &self.mac_addr)
            .finish()
    }
}

// ============================================================================
// Interrupt Handler
// ============================================================================

/// Top-level interrupt handler for the e1000e.
/// Reads ICR to clear the interrupt (level-triggered INTx requires this).
fn handle_interrupt(_frame: &TrapFrame) {
    if let Some(regs) = IRQ_REGS.get() {
        let icr = regs.read(ICR);
        if icr == 0 {
            return; // Not our interrupt
        }
    }
    aster_network::raise_receive_softirq();
    aster_network::raise_send_softirq();
}

// ============================================================================
// Hardware helpers
// ============================================================================

/// Performs a full hardware reset of the 82574L.
fn hw_reset(regs: &E1000eRegs) {
    // Mask all interrupts
    regs.write(IMC, 0xFFFF_FFFF);

    // Disable RX and TX
    regs.write(RCTL, 0);
    regs.write(TCTL, TCTL_PSP);

    // Flush
    let _ = regs.read(STATUS);

    // Delay
    delay_us(10);

    // Issue global reset
    let ctrl = regs.read(CTRL);
    regs.write(CTRL, ctrl | CTRL_RST);

    // Wait for reset to complete
    delay_us(2000);

    // Clear interrupt masks again
    regs.write(IMC, 0xFFFF_FFFF);
    // Clear pending interrupts
    let _ = regs.read(ICR);
}

/// Reads a single word from the EEPROM via the EERD register.
fn eerd_read(regs: &E1000eRegs, offset: u16) -> Result<u16, &'static str> {
    regs.write(EERD, (offset as u32) << EERD_ADDR_SHIFT | EERD_START);

    // Poll for completion
    for _ in 0..1000 {
        let val = regs.read(EERD);
        if val & EERD_DONE != 0 {
            return Ok((val >> EERD_DATA_SHIFT) as u16);
        }
        delay_us(5);
    }

    Err("EERD timeout")
}

/// Reads the MAC address from the EEPROM (3 words at offset 0).
fn read_mac_from_eerd(regs: &E1000eRegs) -> Result<[u8; 6], &'static str> {
    let mut mac = [0u8; 6];
    for i in 0..3u16 {
        let word = eerd_read(regs, i)?;
        mac[i as usize * 2] = (word & 0xFF) as u8;
        mac[i as usize * 2 + 1] = (word >> 8) as u8;
    }
    Ok(mac)
}

/// Programs a receive address into the specified RAR slot.
fn rar_set(regs: &E1000eRegs, addr: &[u8; 6], index: u32) {
    let ral = (addr[0] as u32)
        | ((addr[1] as u32) << 8)
        | ((addr[2] as u32) << 16)
        | ((addr[3] as u32) << 24);
    let rah = (addr[4] as u32) | ((addr[5] as u32) << 8) | RAH_AV;

    regs.write(RA + (index as usize * 8), ral);
    regs.write(RA + (index as usize * 8) + 4, rah);
}

/// Microsecond delay (busy-loop approximation).
fn delay_us(us: u32) {
    for _ in 0..(us as u64 * 100) {
        core::hint::spin_loop();
    }
}
