// SPDX-License-Identifier: MPL-2.0

//! `AnyNetworkDevice` trait implementation for the RTL8168g (r8169) network
//! controller on Asterinas OS.
//!
//! This module wraps the driver state (`Mmio`, `TxRing`, descriptor ring) into
//! the Asterinas `AnyNetworkDevice` interface, exposing `send`, `receive`,
//! `mac_addr`, `capabilities`, and the other trait methods required by the
//! `aster-network` component.
//!
//! The design follows the same patterns used by the virtio-net and e1000
//! drivers in the Asterinas tree:
//!   - A `PciDriver` implementation that probes for Realtek PCI devices.
//!   - A `R8169Device` struct that owns MMIO, TX ring, RX descriptor ring,
//!     and per-slot `RxBuffer` objects from the DMA pool.
//!   - Registration with `aster_network::register_device` at probe time.
//!   - An MSI-X interrupt handler that raises softirqs for RX/TX processing.
//!
//! The RX path programs the DMA address of `RxBuffer`-backed segments into
//! the hardware descriptor ring so the NIC writes received data directly
//! into the pool-allocated buffers.  This avoids an extra copy and follows
//! the same approach used by the e1000 driver.
//!
//! Translated from: drivers/net/ethernet/realtek/r8169_main.c

use alloc::{string::ToString, sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_bigtcp::device::{Checksum, DeviceCapabilities, Medium};
use aster_network::{AnyNetworkDevice, EthernetAddr, NetError, RxBuffer};
use aster_pci::{
    PciDeviceId,
    bus::{PciDevice, PciDriver},
    capability::msix::CapabilityMsixData,
    cfg_space::Bar,
    common_device::PciCommonDevice,
};
use ostd::{
    arch::trap::TrapFrame,
    bus::BusProbeError,
    debug, error, warn,
    irq::IrqLine,
    mm::{HasDaddr, dma::FromDevice},
    sync::SpinLock,
};
use spin::Once;

use crate::desc::{DescRing, RawDesc, is_fragmented_frame};
use crate::regs::{
    self, Mmio, DESC_OWN, ETH_FCS_LEN, INTR_MASK, INTR_STATUS, MAC0, MAC4, NUM_RX_DESC,
    RING_END, RX_RES,
};
use crate::rx;
use crate::tx::{self, TxRing};

// ============================================================================
// Constants
// ============================================================================

/// PCI vendor IDs that may use the r8169 driver.
const SUPPORTED_VENDOR_IDS: &[u16] = &[
    0x10EC, // Realtek
    0x1186, // D-Link
    0x1259, // Allied Telesis
    0x16EC, // US Robotics
    0x1737, // Linksys
];

/// PCI device IDs served by the r8169 family.
const SUPPORTED_DEVICE_IDS: &[u16] = &[
    0x2502, 0x2600, 0x8129, 0x8136, 0x8161, 0x8162, 0x8167, 0x8168, 0x8169,
    0x8125, 0x8126, 0x8127, 0x3000, 0x5000, 0x0e10,
];

/// Network device name registered with the network subsystem.
pub const DEVICE_NAME: &str = "r8169-net";

/// Maximum Ethernet frame payload (standard MTU).
const MTU: usize = 1500;

/// Interrupt event bits that we care about.
const RTL_EVENT_NAPI: u16 =
    regs::RX_OK | regs::TX_OK | regs::RX_OVERFLOW | regs::TX_ERR | regs::RX_ERR;

/// All interrupts we want to enable.
const INTR_MASK_BITS: u16 = RTL_EVENT_NAPI | regs::LINK_CHG;

/// RX DMA pool for the r8169 driver.
static RX_POOL: Once<Arc<aster_network::dma_pool::DmaPool<FromDevice>>> = Once::new();

/// RX buffer segment size for the DMA pool.  Must be large enough for
/// a standard Ethernet frame (1514 bytes payload + 14-byte header + 4-byte
/// FCS = 1532 bytes, rounded up for alignment).  The pool allocates
/// page-sized chunks; segments of 2048 bytes comfortably hold standard
/// frames.  Jumbo frames are not supported in this initial bring-up.
const RX_POOL_SEG_SIZE: usize = 2048;

/// The buffer size value programmed into the RX descriptor's opts1 field.
/// This tells the NIC how many bytes it may write into each buffer.
/// Must not exceed the actual DMA segment size.
const RX_DESC_BUF_SIZE: u32 = RX_POOL_SEG_SIZE as u32;

/// Mask for extracting packet size from opts1 (bits 13:0).
const PKT_SIZE_MASK: u32 = 0x3FFF;

// ============================================================================
// PCI Driver
// ============================================================================

/// PCI driver instance for r8169.
#[derive(Debug)]
pub struct R8169PciDriver;

impl PciDriver for R8169PciDriver {
    fn probe(
        &self,
        device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (BusProbeError, PciCommonDevice)> {
        let vendor_id = device.device_id().vendor_id;
        if !SUPPORTED_VENDOR_IDS.contains(&vendor_id) {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        let dev_id = device.device_id().device_id;
        if !SUPPORTED_DEVICE_IDS.contains(&dev_id) {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        match R8169Device::init(device) {
            Ok(pci_device) => Ok(pci_device),
            Err((err_msg, device)) => {
                error!("r8169: probe failed: {}", err_msg);
                Err((BusProbeError::ConfigurationSpaceError, device))
            }
        }
    }
}

// ============================================================================
// PCI Device wrapper (returned from probe)
// ============================================================================

/// Thin wrapper satisfying `PciDevice` for the claimed r8169.
#[derive(Debug)]
struct R8169PciDeviceWrapper {
    device_id: PciDeviceId,
}

impl PciDevice for R8169PciDeviceWrapper {
    fn device_id(&self) -> PciDeviceId {
        self.device_id
    }
}

// ============================================================================
// R8169 Network Device
// ============================================================================

/// The RTL8168g adapter / network device.
///
/// Owns the MMIO handle, TX descriptor ring, RX descriptor ring (with
/// per-slot `RxBuffer` objects), the MAC address, and device capabilities.
///
/// An instance is wrapped in `Arc<SpinLock<..>>` and registered with the
/// network subsystem.
pub struct R8169Device {
    /// Safe MMIO accessor wrapping the PCI memory BAR.
    mmio: Mmio,
    /// Transmit descriptor ring and associated state.
    tx_ring: TxRing,
    /// RX descriptor ring (DMA-coherent memory).
    rx_desc_ring: DescRing,
    /// Per-slot RX buffers allocated from the DMA pool.
    /// The NIC writes received data directly into these buffers.
    rx_buffers: Vec<Option<RxBuffer>>,
    /// Current RX consumer index.
    rx_cur: u32,
    /// Hardware MAC address.
    mac_addr: EthernetAddr,
    /// Device capabilities (for the network stack / smoltcp).
    caps: DeviceCapabilities,
}

impl R8169Device {
    /// Probes and initializes the RTL8168g device from a `PciCommonDevice`.
    ///
    /// On success, the device is registered with `aster_network` and an
    /// `Arc<dyn PciDevice>` wrapper is returned.  On failure, the
    /// `PciCommonDevice` is returned so the PCI bus can try other drivers.
    fn init(
        mut device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (&'static str, PciCommonDevice)> {
        let device_id = *device.device_id();

        // --- Acquire a memory BAR ---
        // RTL8168g may present MMIO on BAR 2 (memory-mapped) or BAR 0.
        // Try BAR 2 first, then BAR 0.
        let bar_access = {
            let try_bars = [2u8, 0];
            let mut found = None;
            for &idx in &try_bars {
                if let Some(bar) = device.bar_manager_mut().bar_mut(idx) {
                    if let Bar::Memory(mem_bar) = bar {
                        if let Ok(io_mem) = mem_bar.acquire() {
                            found = Some(io_mem.clone());
                            break;
                        }
                    }
                }
            }
            match found {
                Some(access) => aster_pci::cfg_space::BarAccess::Memory(access),
                None => return Err(("No suitable memory BAR found", device)),
            }
        };

        let mmio = Mmio::new(bar_access);

        // --- Set up MSI-X interrupts (if available) ---
        let msix_data = match device.acquire_msix_capability() {
            Ok(Some(msix)) => Some(msix),
            _ => None,
        };

        // --- Initialize the RX DMA pool (once, shared across devices) ---
        RX_POOL.call_once(|| {
            aster_network::dma_pool::DmaPool::new(
                RX_POOL_SEG_SIZE,
                32,  // initial pages
                64,  // high watermark
                false,
            )
        });

        // --- Read the MAC address from hardware ---
        let mac_addr = read_mac_addr(&mmio);

        // --- Create TX ring ---
        let tx_ring = match TxRing::new() {
            Ok(r) => r,
            Err(_) => return Err(("Failed to allocate TX ring", device)),
        };

        // --- Create RX descriptor ring ---
        let rx_desc_ring = match DescRing::new(NUM_RX_DESC) {
            Ok(r) => r,
            Err(_) => return Err(("Failed to allocate RX descriptor ring", device)),
        };

        // --- Allocate RxBuffers from the pool and fill descriptors ---
        let pool = RX_POOL.get().unwrap();
        let mut rx_buffers = Vec::with_capacity(NUM_RX_DESC);
        for i in 0..NUM_RX_DESC {
            let rx_buffer = match RxBuffer::new(0, pool) {
                Ok(buf) => buf,
                Err(_) => return Err(("Failed to allocate RX buffer", device)),
            };

            let dma_addr = rx_buffer.daddr();
            let mut opts1 = RX_DESC_BUF_SIZE | DESC_OWN;
            if i == NUM_RX_DESC - 1 {
                opts1 |= RING_END;
            }

            let desc = RawDesc {
                opts1,
                opts2: 0,
                addr_lo: dma_addr as u32,
                addr_hi: (dma_addr >> 32) as u32,
            };
            if let Err(_) = rx_desc_ring.write_desc(i, &desc) {
                return Err(("Failed to write RX descriptor", device));
            }

            rx_buffers.push(Some(rx_buffer));
        }

        // --- Program TX/RX descriptor addresses into hardware ---
        if let Err(_) = tx::set_tx_desc_addr(&mmio, tx_ring.ring_dma_addr()) {
            return Err(("Failed to set TX descriptor address", device));
        }
        if let Err(_) = rx::set_rx_desc_addr(&mmio, rx_desc_ring.dma_addr()) {
            return Err(("Failed to set RX descriptor address", device));
        }

        // --- Set TX/RX size registers ---
        let _ = tx::set_tx_max_size(&mmio);
        let _ = rx::set_rx_max_size(&mmio);

        // --- Build device capabilities ---
        let mut caps = DeviceCapabilities::default();
        caps.max_burst_size = None;
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = MTU;
        // Software checksums (no hardware offload in initial bring-up).
        caps.checksum.tcp = Checksum::Both;
        caps.checksum.udp = Checksum::Both;
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.icmpv4 = Checksum::Both;

        // --- Enable RX and TX in ChipCmd ---
        let _ = mmio.write8(regs::CHIP_CMD, regs::CMD_RX_ENB | regs::CMD_TX_ENB);

        // --- Enable interrupts ---
        let _ = mmio.write16(INTR_MASK, INTR_MASK_BITS);

        let adapter = R8169Device {
            mmio,
            tx_ring,
            rx_desc_ring,
            rx_buffers,
            rx_cur: 0,
            mac_addr,
            caps,
        };

        // --- Register with the network subsystem ---
        let device_ref = Arc::new(SpinLock::new(adapter));
        aster_network::register_device(DEVICE_NAME.to_string(), device_ref);

        // --- Register MSI-X interrupt handler ---
        if let Some(mut msix) = msix_data {
            register_msix_handler(&mut msix);
        }

        debug!(
            "r8169: Device initialized, MAC = {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac_addr.0[0],
            mac_addr.0[1],
            mac_addr.0[2],
            mac_addr.0[3],
            mac_addr.0[4],
            mac_addr.0[5]
        );

        Ok(Arc::new(R8169PciDeviceWrapper { device_id }))
    }

    /// Allocates a fresh RX buffer from the pool and programs its DMA
    /// address into the given descriptor slot, marking it as owned by
    /// the hardware.
    ///
    /// This is the equivalent of `rtl8169_alloc_rx_data` in the C driver,
    /// adapted to use the DMA pool's `RxBuffer`.
    fn alloc_rx_slot(&mut self, index: usize) -> Result<(), NetError> {
        let pool = RX_POOL.get().ok_or(NetError::NotReady)?;
        let rx_buffer = RxBuffer::new(0, pool).map_err(|_| NetError::NoMemory)?;

        let dma_addr = rx_buffer.daddr();
        let mut opts1 = RX_DESC_BUF_SIZE | DESC_OWN;
        if index == NUM_RX_DESC - 1 {
            opts1 |= RING_END;
        }

        let desc = RawDesc {
            opts1,
            opts2: 0,
            addr_lo: dma_addr as u32,
            addr_hi: (dma_addr >> 32) as u32,
        };
        self.rx_desc_ring
            .write_desc(index, &desc)
            .map_err(|_| NetError::NoMemory)?;

        self.rx_buffers[index] = Some(rx_buffer);
        Ok(())
    }
}

// ============================================================================
// AnyNetworkDevice implementation
// ============================================================================

impl AnyNetworkDevice for R8169Device {
    fn mac_addr(&self) -> EthernetAddr {
        self.mac_addr
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.caps.clone()
    }

    fn can_receive(&self) -> bool {
        let entry = (self.rx_cur as usize) % NUM_RX_DESC;
        match self.rx_desc_ring.read_opts1(entry) {
            Ok(status) => status & DESC_OWN == 0,
            Err(_) => false,
        }
    }

    fn can_send(&self) -> bool {
        self.tx_ring.can_send()
    }

    /// Receives a single packet from the network.
    ///
    /// Polls the next RX descriptor.  If the NIC has completed writing a
    /// packet, the corresponding `RxBuffer` is extracted, its payload length
    /// is set, a fresh buffer is allocated for the descriptor slot, and the
    /// completed buffer is returned to the caller.
    ///
    /// This follows the same pattern as the e1000 driver's `clean_rx_irq`.
    fn receive(&mut self) -> Result<RxBuffer, NetError> {
        let entry = (self.rx_cur as usize) % NUM_RX_DESC;

        // Read the descriptor status (opts1).
        let status = self
            .rx_desc_ring
            .read_opts1(entry)
            .map_err(|_| NetError::NotReady)?;

        // If the descriptor is still owned by hardware, nothing to receive.
        if status & DESC_OWN != 0 {
            return Err(NetError::NotReady);
        }

        // Check for RX errors.
        if status & RX_RES != 0 {
            // Re-arm the descriptor with a fresh buffer.
            let _ = self.alloc_rx_slot(entry);
            self.rx_cur = self.rx_cur.wrapping_add(1);
            return Err(NetError::NotReady);
        }

        // Reject fragmented frames (first+last must both be set).
        if is_fragmented_frame(status) {
            let _ = self.alloc_rx_slot(entry);
            self.rx_cur = self.rx_cur.wrapping_add(1);
            return Err(NetError::NotReady);
        }

        // Extract packet size from bits 13:0.
        let raw_size = (status & PKT_SIZE_MASK) as usize;
        let pkt_size = if raw_size > ETH_FCS_LEN {
            raw_size - ETH_FCS_LEN
        } else {
            // Degenerate frame -- recycle.
            let _ = self.alloc_rx_slot(entry);
            self.rx_cur = self.rx_cur.wrapping_add(1);
            return Err(NetError::NotReady);
        };

        // Take the completed buffer out of the slot.
        let mut rx_buffer = match self.rx_buffers[entry].take() {
            Some(buf) => buf,
            None => {
                let _ = self.alloc_rx_slot(entry);
                self.rx_cur = self.rx_cur.wrapping_add(1);
                return Err(NetError::NotReady);
            }
        };

        // Set the payload length so the consumer can read the data.
        rx_buffer.set_payload_len(pkt_size);

        // Allocate a fresh buffer for this descriptor slot.
        let _ = self.alloc_rx_slot(entry);

        // Advance the consumer index.
        self.rx_cur = self.rx_cur.wrapping_add(1);

        Ok(rx_buffer)
    }

    fn send(&mut self, packet: &[u8]) -> Result<(), NetError> {
        tx::start_xmit(&mut self.tx_ring, &self.mmio, packet)
    }

    fn free_processed_tx_buffers(&mut self) {
        tx::rtl_tx(&mut self.tx_ring);
    }

    fn notify_poll_end(&mut self) {
        // Re-enable interrupts after NAPI-like polling is complete.
        let _ = self.mmio.write16(INTR_MASK, INTR_MASK_BITS);
    }
}

impl Debug for R8169Device {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("R8169Device")
            .field("mac_addr", &self.mac_addr)
            .finish()
    }
}

// ============================================================================
// MAC address reading
// ============================================================================

/// Reads the 6-byte MAC address from the hardware registers (MAC0..MAC4+1).
///
/// Corresponds to reading the permanent station address from RTL registers
/// 0x00..0x05 in the C driver (`rtl_read_mac_address`).
fn read_mac_addr(mmio: &Mmio) -> EthernetAddr {
    let mut addr = [0u8; 6];

    // MAC0 (offset 0x00) holds bytes 0-3 in little-endian order.
    if let Ok(lo) = mmio.read32(MAC0) {
        addr[0] = (lo & 0xFF) as u8;
        addr[1] = ((lo >> 8) & 0xFF) as u8;
        addr[2] = ((lo >> 16) & 0xFF) as u8;
        addr[3] = ((lo >> 24) & 0xFF) as u8;
    }
    // MAC4 (offset 0x04) holds bytes 4-5 in the low 16 bits.
    if let Ok(hi) = mmio.read32(MAC4) {
        addr[4] = (hi & 0xFF) as u8;
        addr[5] = ((hi >> 8) & 0xFF) as u8;
    }

    // If the address is all zeros or all 0xFF, use a fallback.
    if addr == [0; 6] || addr == [0xFF; 6] {
        warn!("r8169: Invalid MAC address read from hardware, using fallback");
        addr = [0x52, 0x54, 0x00, 0x12, 0x34, 0x57];
    }

    EthernetAddr(addr)
}

// ============================================================================
// Interrupt handling
// ============================================================================

/// Registers MSI-X interrupt handlers that raise softirqs for RX and TX
/// processing.
///
/// Allocates an IRQ line, programs it into MSI-X vector 0, and registers
/// a callback that raises both RX and TX softirqs.
fn register_msix_handler(msix: &mut CapabilityMsixData) {
    if msix.table_size() == 0 {
        return;
    }

    // Allocate an IRQ line and program it into MSI-X vector 0.
    let irq = match IrqLine::alloc() {
        Ok(irq) => irq,
        Err(_) => {
            warn!("r8169: Failed to allocate IRQ line for MSI-X");
            return;
        }
    };
    msix.set_interrupt_vector(irq, 0);

    // Register our interrupt handler on the allocated vector.
    if let Some(irq) = msix.irq_mut(0) {
        irq.on_active(handle_interrupt);
    }
}

/// Top-level interrupt handler for the RTL8168g.
///
/// Called from the IRQ framework in interrupt context.  Raises the network
/// softirqs so that RX and TX processing happens in softirq context
/// (matching the NAPI model from the Linux driver).
fn handle_interrupt(_frame: &TrapFrame) {
    aster_network::raise_receive_softirq();
    aster_network::raise_send_softirq();
}

// ============================================================================
// Interrupt mask helpers
// ============================================================================

/// Disables all RTL8168g interrupts by clearing the IntrMask register.
///
/// Corresponds to `rtl_irq_disable` in the C driver.
pub fn irq_disable(mmio: &Mmio) {
    let _ = mmio.write16(INTR_MASK, 0);
}

/// Enables the standard set of RTL8168g interrupts.
///
/// Corresponds to `rtl_irq_enable` in the C driver.
pub fn irq_enable(mmio: &Mmio) {
    let _ = mmio.write16(INTR_MASK, INTR_MASK_BITS);
}

/// Acknowledges (clears) pending interrupts by writing to IntrStatus.
///
/// Returns the status bits that were pending.
///
/// Corresponds to reading + writing IntrStatus in the C driver's NAPI poll.
pub fn irq_ack(mmio: &Mmio) -> u16 {
    let status = mmio.read16(INTR_STATUS).unwrap_or(0);
    if status != 0 {
        let _ = mmio.write16(INTR_STATUS, status);
    }
    status
}
