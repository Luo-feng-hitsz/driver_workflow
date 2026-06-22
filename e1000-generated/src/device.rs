// SPDX-License-Identifier: MPL-2.0

//! E1000 network device implementation.
//!
//! Implements the `AnyNetworkDevice` trait from `aster-network` using
//! `DmaCoherent` for descriptor rings and `DmaPool` for packet buffers.

use alloc::{string::ToString, sync::Arc, vec::Vec};
use core::fmt::Debug;

use aster_bigtcp::device::{Checksum, DeviceCapabilities, Medium};
use aster_network::{AnyNetworkDevice, EthernetAddr, NetError, RxBuffer, TxBuffer};
use aster_pci::{
    PciDeviceLocation,
    capability::msix::CapabilityMsixData,
    cfg_space::PciCommonCfgOffset,
};
use ostd::{
    arch::{
        irq::{IRQ_CHIP, MappedIrqLine},
        trap::TrapFrame,
    },
    io::IoMem,
    irq::IrqLine,
    mm::{HasDaddr, HasSize, VmIoOnce, dma::DmaCoherent},
    sync::SpinLock,
};

use crate::{
    buffer::{RX_BUFFER_POOL, TX_BUFFER_POOL},
    desc::*,
    hw,
    regs::*,
};

pub const DEVICE_NAME: &str = "E1000";

pub struct E1000Device {
    io_mem: IoMem,
    mac_addr: EthernetAddr,
    caps: DeviceCapabilities,

    /// DMA-coherent memory backing the RX descriptor ring.
    rx_ring: DmaCoherent,
    /// DMA-coherent memory backing the TX descriptor ring.
    tx_ring: DmaCoherent,

    /// Data buffers currently posted to the RX ring, indexed by descriptor slot.
    rx_buffers: Vec<Option<RxBuffer>>,
    /// Data buffers currently posted to the TX ring, indexed by descriptor slot.
    tx_buffers: Vec<Option<TxBuffer>>,

    rx_tail: u16,
    tx_tail: u16,

    /// IRQ line kept alive so the callback stays registered.
    _irq: IrqHolder,
}

enum IrqHolder {
    None,
    Msix(#[expect(dead_code)] IrqLine),
    Legacy(#[expect(dead_code)] MappedIrqLine),
}

// =============================================================================
// Initialization
// =============================================================================

impl E1000Device {
    pub(crate) fn init(
        io_mem: IoMem,
        mut msix: Option<CapabilityMsixData>,
        location: PciDeviceLocation,
    ) -> Result<(), E1000Error> {
        // 1. Reset device
        hw::reset_device(&io_mem);

        // 2. Read MAC address
        let mac_addr = hw::read_mac_address(&io_mem);
        ostd::info!(
            "MAC address: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac_addr.0[0],
            mac_addr.0[1],
            mac_addr.0[2],
            mac_addr.0[3],
            mac_addr.0[4],
            mac_addr.0[5],
        );

        // 3. Clear multicast table
        hw::clear_multicast_table(&io_mem);

        // 4. Setup flow control
        hw::setup_flow_control(&io_mem);

        // 5. Setup interrupts
        let irq = Self::setup_interrupts(&io_mem, &mut msix, &location);

        // 6. Setup RX ring
        let rx_pool = RX_BUFFER_POOL.get().unwrap();
        let (rx_ring, rx_buffers) =
            alloc_rx_ring(rx_pool).map_err(|_| E1000Error::ResourceAlloc)?;
        hw::setup_rx_hardware(&io_mem, rx_ring.daddr() as u64, NUM_RX_DESCS as usize);

        // 7. Setup TX ring
        let (tx_ring, tx_buffers) = alloc_tx_ring().map_err(|_| E1000Error::ResourceAlloc)?;
        hw::setup_tx_hardware(&io_mem, tx_ring.daddr() as u64, NUM_TX_DESCS as usize);

        // 8. Link up
        hw::setup_link(&io_mem);

        let device = Self {
            io_mem,
            mac_addr,
            caps: Self::init_capabilities(),
            rx_ring,
            tx_ring,
            rx_buffers,
            tx_buffers,
            rx_tail: NUM_RX_DESCS - 1,
            tx_tail: 0,
            _irq: irq,
        };

        aster_network::register_device(
            DEVICE_NAME.to_string(),
            Arc::new(SpinLock::new(device)),
        );
        Ok(())
    }

    fn setup_interrupts(
        io_mem: &IoMem,
        msix: &mut Option<CapabilityMsixData>,
        location: &PciDeviceLocation,
    ) -> IrqHolder {
        // Try MSI-X first
        if let Some(msix_data) = msix.as_mut() {
            if let Ok(mut irq) = IrqLine::alloc() {
                irq.on_active(|_: &TrapFrame| {
                    aster_network::raise_receive_softirq();
                    aster_network::raise_send_softirq();
                });
                msix_data.set_interrupt_vector(irq.clone(), 0);
                return IrqHolder::Msix(irq);
            }
        }

        // Fallback: legacy INTx via IOAPIC
        let irq_pin = location.read8(PciCommonCfgOffset::InterruptPin as u16);
        if irq_pin == 0 {
            ostd::warn!("e1000: device has no interrupt pin, polling only");
            return IrqHolder::None;
        }
        let irq_line_num = location.read8(PciCommonCfgOffset::InterruptLine as u16);
        ostd::info!("e1000: using legacy interrupt, GSI {}", irq_line_num);

        let irq_line = match IrqLine::alloc() {
            Ok(irq) => irq,
            Err(_) => {
                ostd::warn!("e1000: failed to allocate IRQ line, polling only");
                return IrqHolder::None;
            }
        };

        let irq_chip = match IRQ_CHIP.get() {
            Some(chip) => chip,
            None => {
                ostd::warn!("e1000: no IRQ chip available, polling only");
                return IrqHolder::None;
            }
        };

        match irq_chip.map_gsi_pin_to(irq_line, irq_line_num as u32) {
            Ok(mut mapped) => {
                // Legacy INTx is level-triggered: read ICR to deassert.
                let icr_io = io_mem.slice(REG_ICR..REG_ICR + 4);
                mapped.on_active(move |_: &TrapFrame| {
                    // Reading ICR clears the interrupt cause.
                    let _: u32 = icr_io.read_once(0).unwrap();
                    aster_network::raise_receive_softirq();
                    aster_network::raise_send_softirq();
                });
                // Enable interrupt sources after the handler is in place
                hw::enable_interrupts(io_mem);
                IrqHolder::Legacy(mapped)
            }
            Err(_) => {
                ostd::warn!("e1000: failed to map GSI {}, polling only", irq_line_num);
                IrqHolder::None
            }
        }
    }

    fn init_capabilities() -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.max_burst_size = None;
        caps.medium = Medium::Ethernet;
        // Standard Ethernet MTU (header + payload)
        caps.max_transmission_unit = 1514;
        // We do not offload checksums; stack must compute them.
        caps.checksum.tcp = Checksum::Both;
        caps.checksum.udp = Checksum::Both;
        caps.checksum.ipv4 = Checksum::Both;
        caps.checksum.icmpv4 = Checksum::Both;
        caps
    }
}

// =============================================================================
// AnyNetworkDevice Implementation
// =============================================================================

impl AnyNetworkDevice for E1000Device {
    fn mac_addr(&self) -> EthernetAddr {
        self.mac_addr
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.caps.clone()
    }

    fn can_receive(&self) -> bool {
        let next = (self.rx_tail + 1) % NUM_RX_DESCS;
        let desc = read_rx_desc(&self.rx_ring, next);
        desc.status & RXD_STAT_DD != 0
    }

    fn can_send(&self) -> bool {
        let next_tail = (self.tx_tail + 1) % NUM_TX_DESCS;
        // Check if the next slot's descriptor has been consumed (DD set) or is empty
        let desc = read_tx_desc(&self.tx_ring, self.tx_tail);
        self.tx_buffers[self.tx_tail as usize].is_none()
            || desc.status & TXD_STAT_DD != 0
            || next_tail != hw::read_reg(&self.io_mem, REG_TDH) as u16
    }

    fn receive(&mut self) -> Result<RxBuffer, NetError> {
        let rx_pool = RX_BUFFER_POOL.get().unwrap();
        let next = (self.rx_tail + 1) % NUM_RX_DESCS;

        // Read descriptor to check if hardware has written data
        let desc = read_rx_desc(&self.rx_ring, next);
        if desc.status & RXD_STAT_DD == 0 {
            return Err(NetError::NotReady);
        }

        // Take the completed buffer
        let mut rx_buffer = self.rx_buffers[next as usize]
            .take()
            .ok_or(NetError::NotReady)?;
        rx_buffer.set_payload_len(desc.length as usize);

        // Allocate a replacement buffer
        let new_buffer = RxBuffer::new(0, rx_pool).map_err(|_| NetError::NoMemory)?;

        // Rewrite the descriptor with the new buffer's DMA address
        let new_desc = RxDesc {
            buffer_addr: new_buffer.daddr() as u64,
            ..RxDesc::default()
        };
        write_rx_desc(&self.rx_ring, next, &new_desc);
        self.rx_buffers[next as usize] = Some(new_buffer);

        // Advance tail - tells hardware it can use this descriptor again
        self.rx_tail = next;
        hw::write_reg(&self.io_mem, REG_RDT, self.rx_tail as u32);

        Ok(rx_buffer)
    }

    fn send(&mut self, packet: &[u8]) -> Result<(), NetError> {
        let idx = self.tx_tail;
        let next_tail = (idx + 1) % NUM_TX_DESCS;

        // Check ring full: next_tail must not equal hardware head
        if next_tail == hw::read_reg(&self.io_mem, REG_TDH) as u16 {
            return Err(NetError::Busy);
        }

        let tx_pool = TX_BUFFER_POOL.get().unwrap();

        // e1000 sends raw Ethernet frames - no hardware header needed
        let tx_buffer =
            TxBuffer::new(&(), packet, tx_pool).map_err(|_| NetError::NoMemory)?;

        // Fill descriptor
        let desc = TxDesc {
            buffer_addr: tx_buffer.daddr() as u64,
            length: tx_buffer.size() as u16,
            cmd: (TxCmd::EOP | TxCmd::IFCS | TxCmd::RS).bits(),
            ..TxDesc::default()
        };
        write_tx_desc(&self.tx_ring, idx, &desc);

        // Keep buffer alive until hardware is done with it
        self.tx_buffers[idx as usize] = Some(tx_buffer);

        // Advance tail - triggers hardware to start transmitting
        self.tx_tail = next_tail;
        hw::write_reg(&self.io_mem, REG_TDT, self.tx_tail as u32);

        Ok(())
    }

    fn free_processed_tx_buffers(&mut self) {
        for i in 0..NUM_TX_DESCS {
            let idx = i as usize;
            if self.tx_buffers[idx].is_none() {
                continue;
            }
            let desc = read_tx_desc(&self.tx_ring, i);
            if desc.status & TXD_STAT_DD != 0 {
                self.tx_buffers[idx] = None;
            }
        }
    }

    fn notify_poll_end(&mut self) {
        // No batched notification needed for e1000.
    }
}

impl Debug for E1000Device {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("E1000Device")
            .field("mac_addr", &self.mac_addr)
            .field("rx_tail", &self.rx_tail)
            .field("tx_tail", &self.tx_tail)
            .finish()
    }
}

// =============================================================================
// Error
// =============================================================================

#[derive(Debug)]
pub(crate) enum E1000Error {
    ResourceAlloc,
}
