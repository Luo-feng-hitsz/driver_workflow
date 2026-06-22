// SPDX-License-Identifier: MPL-2.0

//! TX/RX descriptor ring types, ring constants, ring index management,
//! DMA allocation for descriptor rings, and ring fill/clear operations.
//!
//! Translated from: drivers/net/ethernet/realtek/r8169_main.c

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use ostd::mm::{
    Daddr, FrameAllocOptions, HasDaddr,
    dma::{DmaDirection, DmaStream, FromAndToDevice},
    io::util::HasVmReaderWriter,
};

use crate::regs::{
    DESC_OWN, FIRST_FRAG, LAST_FRAG, NUM_RX_DESC, NUM_TX_DESC, R8169_RX_BUF_SIZE, RING_END,
};

/// Size of a single descriptor in bytes.
pub const DESC_SIZE: usize = core::mem::size_of::<RawDesc>();

/// Size of the TX descriptor ring in bytes.
pub const TX_RING_BYTES: usize = NUM_TX_DESC * DESC_SIZE;

/// Size of the RX descriptor ring in bytes.
pub const RX_RING_BYTES: usize = NUM_RX_DESC * DESC_SIZE;

/// Raw hardware descriptor layout (identical for TX and RX).
/// Must be 256-byte aligned (provided by DMA coherent allocation).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RawDesc {
    pub opts1: u32,
    pub opts2: u32,
    pub addr_lo: u32,
    pub addr_hi: u32,
}

impl RawDesc {
    pub const fn zeroed() -> Self {
        Self {
            opts1: 0,
            opts2: 0,
            addr_lo: 0,
            addr_hi: 0,
        }
    }
}

/// Ring index state shared between the driver and hardware paths.
pub struct RingIndexes {
    pub cur_tx: AtomicU32,
    pub dirty_tx: AtomicU32,
    pub cur_rx: AtomicU32,
}

impl RingIndexes {
    pub fn new() -> Self {
        Self {
            cur_tx: AtomicU32::new(0),
            dirty_tx: AtomicU32::new(0),
            cur_rx: AtomicU32::new(0),
        }
    }

    pub fn reset(&self) {
        self.cur_tx.store(0, Ordering::Release);
        self.dirty_tx.store(0, Ordering::Release);
        self.cur_rx.store(0, Ordering::Release);
    }
}

/// A descriptor ring backed by DMA-coherent memory.
pub struct DescRing {
    /// The DMA stream that backs the descriptor ring.
    dma: Arc<DmaStream<FromAndToDevice>>,
    /// Number of descriptors in the ring.
    count: usize,
}

impl DescRing {
    /// Allocates a new descriptor ring with `count` entries.
    ///
    /// The ring is backed by DMA-coherent pages suitable for device access.
    pub fn new(count: usize) -> Result<Self, ostd::Error> {
        let total_bytes = count * DESC_SIZE;
        let n_pages = (total_bytes + ostd::mm::PAGE_SIZE - 1) / ostd::mm::PAGE_SIZE;
        let segment = FrameAllocOptions::new().alloc_segment(n_pages)?;
        let dma = DmaStream::<FromAndToDevice>::map(segment.into(), true)?;

        // Zero out all descriptors
        {
            let mut writer = dma.writer()?;
            let zeroes = alloc::vec![0u8; total_bytes];
            let mut reader = ostd::mm::VmReader::from(zeroes.as_slice());
            writer.write(&mut reader);
        }
        dma.sync_to_device(0..total_bytes)?;

        Ok(Self {
            dma: Arc::new(dma),
            count,
        })
    }

    /// Returns the DMA (bus) address of the ring start.
    pub fn dma_addr(&self) -> Daddr {
        self.dma.daddr()
    }

    /// Returns the number of descriptors.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Writes a descriptor at the given index.
    pub fn write_desc(&self, index: usize, desc: &RawDesc) -> Result<(), ostd::Error> {
        let offset = index * DESC_SIZE;
        let mut bytes = [0u8; DESC_SIZE];
        bytes[0..4].copy_from_slice(&desc.opts1.to_le_bytes());
        bytes[4..8].copy_from_slice(&desc.opts2.to_le_bytes());
        bytes[8..12].copy_from_slice(&desc.addr_lo.to_le_bytes());
        bytes[12..16].copy_from_slice(&desc.addr_hi.to_le_bytes());
        let mut writer = self.dma.writer()?;
        writer.skip(offset);
        let mut reader = ostd::mm::VmReader::from(&bytes as &[u8]);
        writer.write(&mut reader);
        self.dma.sync_to_device(offset..offset + DESC_SIZE)?;
        Ok(())
    }

    /// Reads a descriptor at the given index.
    pub fn read_desc(&self, index: usize) -> Result<RawDesc, ostd::Error> {
        let offset = index * DESC_SIZE;
        self.dma.sync_from_device(offset..offset + DESC_SIZE)?;
        let mut reader = self.dma.reader()?;
        reader.skip(offset);
        let mut bytes = [0u8; DESC_SIZE];
        let mut writer = ostd::mm::VmWriter::from(&mut bytes as &mut [u8]);
        reader.read(&mut writer);
        Ok(RawDesc {
            opts1: u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            opts2: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            addr_lo: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            addr_hi: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        })
    }

    /// Writes only the opts1 field of a descriptor (for returning to HW).
    pub fn write_opts1(&self, index: usize, opts1: u32) -> Result<(), ostd::Error> {
        let offset = index * DESC_SIZE;
        let bytes = opts1.to_le_bytes();
        let mut writer = self.dma.writer()?;
        writer.skip(offset);
        let mut reader = ostd::mm::VmReader::from(&bytes as &[u8]);
        writer.write(&mut reader);
        self.dma.sync_to_device(offset..offset + 4)?;
        Ok(())
    }

    /// Reads only the opts1 field of a descriptor.
    pub fn read_opts1(&self, index: usize) -> Result<u32, ostd::Error> {
        let offset = index * DESC_SIZE;
        self.dma.sync_from_device(offset..offset + 4)?;
        let mut reader = self.dma.reader()?;
        reader.skip(offset);
        let mut bytes = [0u8; 4];
        let mut writer = ostd::mm::VmWriter::from(&mut bytes as &mut [u8]);
        reader.read(&mut writer);
        Ok(u32::from_le_bytes(bytes))
    }
}

impl HasDaddr for DescRing {
    fn daddr(&self) -> Daddr {
        self.dma.daddr()
    }
}

/// TX buffer metadata associated with each TX descriptor slot.
pub struct TxSlot {
    /// Length of data mapped for this slot.
    pub len: u32,
    /// Whether this slot holds the last fragment (and therefore the packet data to free).
    pub is_last: bool,
    /// Optional reference to the DMA stream for the data buffer.
    pub dma_buf: Option<Arc<DmaStream<ostd::mm::dma::ToDevice>>>,
}

impl TxSlot {
    pub fn new() -> Self {
        Self {
            len: 0,
            is_last: false,
            dma_buf: None,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.is_last = false;
        self.dma_buf = None;
    }
}

/// RX buffer associated with each RX descriptor slot.
pub struct RxSlot {
    /// DMA stream for the receive buffer.
    pub dma_buf: Option<Arc<DmaStream<ostd::mm::dma::FromDevice>>>,
}

impl RxSlot {
    pub fn new() -> Self {
        Self { dma_buf: None }
    }

    pub fn clear(&mut self) {
        self.dma_buf = None;
    }
}

/// Marks an RX descriptor as owned by the hardware.
///
/// Preserves the RingEnd bit if present.
pub fn mark_to_asic(ring: &DescRing, index: usize) -> Result<(), ostd::Error> {
    let opts1 = ring.read_opts1(index)?;
    let eor = opts1 & RING_END;
    // Write opts2 = 0
    let offset = index * DESC_SIZE + 4;
    let zero_bytes = 0u32.to_le_bytes();
    {
        let mut writer = ring.dma.writer()?;
        writer.skip(offset);
        let mut reader = ostd::mm::VmReader::from(&zero_bytes as &[u8]);
        writer.write(&mut reader);
    }
    // Write opts1 with DescOwn | eor | buf_size
    let new_opts1 = DESC_OWN | eor | R8169_RX_BUF_SIZE;
    ring.write_opts1(index, new_opts1)?;
    Ok(())
}

/// Sets the RingEnd bit on the last descriptor in a ring.
pub fn set_ring_end(ring: &DescRing, count: usize) -> Result<(), ostd::Error> {
    if count == 0 {
        return Ok(());
    }
    let last = count - 1;
    let opts1 = ring.read_opts1(last)?;
    ring.write_opts1(last, opts1 | RING_END)?;
    Ok(())
}

/// Returns the number of available TX slots.
pub fn tx_slots_avail(dirty_tx: u32, cur_tx: u32) -> usize {
    (dirty_tx.wrapping_add(NUM_TX_DESC as u32).wrapping_sub(cur_tx)) as usize
}

/// Checks if a received frame is fragmented (not first+last).
pub fn is_fragmented_frame(status: u32) -> bool {
    (status & (FIRST_FRAG | LAST_FRAG)) != (FIRST_FRAG | LAST_FRAG)
}

// ---------------------------------------------------------------------------
// Hardware tally counters (DMA-mapped, filled by CounterDump command)
// ---------------------------------------------------------------------------

/// Hardware tally counters, filled by the NIC via DMA when issuing a
/// `CounterDump` command.  The layout must match the hardware exactly.
///
/// Fields up through `tx_underrun` are present on RTL8168g; the remaining
/// fields (marked "new since RTL8125") are included for forward-compatibility
/// but will read as zero on RTL8168g.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rtl8169Counters {
    pub tx_packets: u64,
    pub rx_packets: u64,
    pub tx_errors: u64,
    pub rx_errors: u32,
    pub rx_missed: u16,
    pub align_errors: u16,
    pub tx_one_collision: u32,
    pub tx_multi_collision: u32,
    pub rx_unicast: u64,
    pub rx_broadcast: u64,
    pub rx_multicast: u32,
    pub tx_aborted: u16,
    pub tx_underrun: u16,
    // -- new since RTL8125 --
    pub tx_octets: u64,
    pub rx_octets: u64,
    pub rx_multicast64: u64,
    pub tx_unicast64: u64,
    pub tx_broadcast64: u64,
    pub tx_multicast64: u64,
    pub tx_pause_on: u32,
    pub tx_pause_off: u32,
    pub tx_pause_all: u32,
    pub tx_deferred: u32,
    pub tx_late_collision: u32,
    pub tx_all_collision: u32,
    pub tx_aborted32: u32,
    pub align_errors32: u32,
    pub rx_frame_too_long: u32,
    pub rx_runt: u32,
    pub rx_pause_on: u32,
    pub rx_pause_off: u32,
    pub rx_pause_all: u32,
    pub rx_unknown_opcode: u32,
    pub rx_mac_error: u32,
    pub tx_underrun32: u32,
    pub rx_mac_missed: u32,
    pub rx_tcam_dropped: u32,
    pub tdu: u32,
    pub rdu: u32,
}

/// Size of the counters structure in bytes (for DMA allocation).
pub const COUNTERS_SIZE: usize = core::mem::size_of::<Rtl8169Counters>();

// ---------------------------------------------------------------------------
// TC (traffic-control) offsets for baseline statistics
// ---------------------------------------------------------------------------

/// Baseline offsets captured once after the first counter dump, so that
/// subsequent reads can report deltas.
#[derive(Clone, Copy, Debug, Default)]
pub struct TcOffsets {
    pub inited: bool,
    pub tx_errors: u64,
    pub tx_multi_collision: u32,
    pub tx_aborted: u16,
    pub rx_missed: u16,
}
