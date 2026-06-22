// SPDX-License-Identifier: MPL-2.0

//! TX and RX descriptor ring structures for the Intel 82574L (e1000e).
//!
//! Defines the hardware descriptor layouts used by the 82574L NIC for transmit
//! and receive operations. The 82574L uses legacy TX descriptors (`TxDesc`,
//! 16 bytes), extended RX descriptors (`RxDescExt`, 16 bytes), context
//! descriptors (`ContextDesc`, 16 bytes) for checksum/TSO offload, and data
//! descriptors (`DataDesc`, 16 bytes) for offloaded data paths.
//!
//! Descriptor rings are backed by `DmaCoherent` memory. Packet data buffers
//! are allocated from `DmaPool` via `RxBuffer`/`TxBuffer` from aster-network.

use alloc::{sync::Arc, vec::Vec};

use aster_network::{RxBuffer, TxBuffer, dma_pool::DmaPool};
use ostd::mm::{
    HasDaddr, VmIo,
    dma::{DmaCoherent, FromDevice},
};
use ostd_pod::Pod;

// =============================================================================
// Constants
// =============================================================================

/// Size of a single legacy TX descriptor in bytes.
pub(crate) const TX_DESC_SIZE: usize = 16;

/// Size of a single extended RX descriptor in bytes.
pub(crate) const RX_DESC_EXT_SIZE: usize = 16;

/// Size of a single context descriptor in bytes.
pub(crate) const CONTEXT_DESC_SIZE: usize = 16;

/// Size of a single data (offload) descriptor in bytes.
pub(crate) const DATA_DESC_SIZE: usize = 16;

// =============================================================================
// TX Descriptor Command Bits (legacy, byte-width for the `cmd` field)
// =============================================================================

/// End of Packet -- marks the last descriptor of a packet.
pub(crate) const TXD_CMD_EOP: u8 = 1 << 0;

/// Insert FCS/CRC -- hardware appends Ethernet CRC.
pub(crate) const TXD_CMD_IFCS: u8 = 1 << 1;

/// Insert Checksum -- hardware inserts IP/TCP/UDP checksum.
pub(crate) const TXD_CMD_IC: u8 = 1 << 2;

/// Report Status -- hardware sets `DD` in status when done.
pub(crate) const TXD_CMD_RS: u8 = 1 << 3;

/// Descriptor Extension (0 = legacy format).
pub(crate) const TXD_CMD_DEXT: u8 = 1 << 5;

/// VLAN Packet Enable.
pub(crate) const TXD_CMD_VLE: u8 = 1 << 6;

/// Interrupt Delay Enable.
pub(crate) const TXD_CMD_IDE: u8 = 1 << 7;

// =============================================================================
// TX Descriptor Status Bits
// =============================================================================

/// Descriptor Done -- hardware has processed the descriptor.
pub(crate) const TXD_STAT_DD: u8 = 1 << 0;

// =============================================================================
// RX Extended Descriptor Status/Error Bits (in the 32-bit `status_error` field)
// =============================================================================

/// Descriptor Done.
pub(crate) const RXDEXT_STAT_DD: u32 = 1 << 0;

/// End of Packet.
pub(crate) const RXDEXT_STAT_EOP: u32 = 1 << 1;

/// Ignore Checksum Indication.
pub(crate) const RXDEXT_STAT_IXSM: u32 = 1 << 2;

/// VLAN Packet.
pub(crate) const RXDEXT_STAT_VP: u32 = 1 << 3;

/// UDP Checksum Calculated.
pub(crate) const RXDEXT_STAT_UDPCS: u32 = 1 << 4;

/// TCP Checksum Calculated.
pub(crate) const RXDEXT_STAT_TCPCS: u32 = 1 << 5;

/// Timestamp Taken.
pub(crate) const RXDEXT_STATERR_TST: u32 = 1 << 8;

/// CRC Error.
pub(crate) const RXDEXT_STATERR_CE: u32 = 1 << 24;

/// Symbol Error.
pub(crate) const RXDEXT_STATERR_SE: u32 = 1 << 25;

/// Sequence Error.
pub(crate) const RXDEXT_STATERR_SEQ: u32 = 1 << 26;

/// Carrier Extension Error.
pub(crate) const RXDEXT_STATERR_CXE: u32 = 1 << 28;

/// RX Data Error.
pub(crate) const RXDEXT_STATERR_RXE: u32 = 1 << 31;

/// Mask for frame errors in extended RX descriptors.
pub(crate) const RXDEXT_ERR_FRAME_MASK: u32 = RXDEXT_STATERR_CE
    | RXDEXT_STATERR_SE
    | RXDEXT_STATERR_SEQ
    | RXDEXT_STATERR_CXE
    | RXDEXT_STATERR_RXE;

// =============================================================================
// Data Descriptor Packet Options (popts field)
// =============================================================================

/// Insert IP checksum.
pub(crate) const TXD_POPTS_IXSM: u8 = 0x01;

/// Insert TCP/UDP checksum.
pub(crate) const TXD_POPTS_TXSM: u8 = 0x02;

// =============================================================================
// Descriptor Structures (repr(C), 16 bytes each)
// =============================================================================

/// Legacy Transmit Descriptor (16 bytes).
///
/// Software fills `buffer_addr`, `length`, `cso`, and `cmd`;
/// hardware writes back `status` after transmission.
///
/// Corresponds to `struct e1000_tx_desc` in the Linux driver.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct TxDesc {
    /// Physical address of the data buffer.
    pub buffer_addr: u64,
    /// Data buffer length in bytes.
    pub length: u16,
    /// Checksum offset -- byte offset where checksum is inserted.
    pub cso: u8,
    /// Command bits (see `TXD_CMD_*` constants).
    pub cmd: u8,
    /// Status bits written back by hardware (see `TXD_STAT_*`).
    pub status: u8,
    /// Checksum start -- byte offset to begin computing checksum.
    pub css: u8,
    /// Special field (VLAN tag when `TXD_CMD_VLE` is set).
    pub special: u16,
}

/// Extended Receive Descriptor -- writeback layout (16 bytes).
///
/// The 82574L uses extended RX descriptors. In the "read" format software
/// provides `buffer_addr` and hardware fills in the "writeback" fields after
/// DMA-ing packet data. This struct represents the writeback layout, which is
/// what software reads after an interrupt to process received packets.
///
/// Corresponds to the `.wb` member of `union e1000_rx_desc_extended` in the
/// Linux driver.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct RxDescExt {
    /// Multiple Receive Queues filter information (lower.mrq).
    pub mrq: u32,
    /// RSS hash or IP-id/checksum union (lower.hi_dword).
    pub rss: u32,
    /// Extended status and error flags (upper.status_error).
    pub status_error: u32,
    /// Received packet length (upper.length).
    pub length: u16,
    /// VLAN tag (upper.vlan), valid when `RXDEXT_STAT_VP` is set.
    pub vlan: u16,
}

/// Offload Context Descriptor (16 bytes).
///
/// Sets up checksum/TSO offload context for subsequent data descriptors.
/// Corresponds to `struct e1000_context_desc` in the Linux driver.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct ContextDesc {
    /// IP checksum start, offset, and end (packed as `lower_setup`).
    pub lower_setup: u32,
    /// TCP/UDP checksum start, offset, and end (packed as `upper_setup`).
    pub upper_setup: u32,
    /// Command and length field.
    pub cmd_and_length: u32,
    /// TCP segmentation setup: status, header length, MSS.
    pub tcp_seg_setup: u32,
}

/// Offload Data Descriptor (16 bytes).
///
/// Used in conjunction with `ContextDesc` for hardware checksum/TSO offload.
/// Corresponds to `struct e1000_data_desc` in the Linux driver.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct DataDesc {
    /// Physical address of the data buffer.
    pub buffer_addr: u64,
    /// Lower word: length (16 bits), type/length extension (8 bits), command (8 bits).
    pub lower: u32,
    /// Upper word: status (8 bits), packet options (8 bits), special/VLAN (16 bits).
    pub upper: u32,
}

// Compile-time size checks.
const _: () = assert!(size_of::<TxDesc>() == TX_DESC_SIZE);
const _: () = assert!(size_of::<RxDescExt>() == RX_DESC_EXT_SIZE);
const _: () = assert!(size_of::<ContextDesc>() == CONTEXT_DESC_SIZE);
const _: () = assert!(size_of::<DataDesc>() == DATA_DESC_SIZE);

// =============================================================================
// RX Descriptor Read Format (internal helper)
// =============================================================================

/// The "read" format of an extended RX descriptor as written by software.
///
/// Hardware reads `buffer_addr` to determine where to DMA the incoming packet.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
struct RxDescRead {
    buffer_addr: u64,
    reserved: u64,
}

const _: () = assert!(size_of::<RxDescRead>() == RX_DESC_EXT_SIZE);

// =============================================================================
// Descriptor Ring
// =============================================================================

/// A descriptor ring backed by DMA-coherent memory.
///
/// Manages a fixed-size array of hardware descriptors along with per-slot
/// buffer metadata (`B` is typically `RxBuffer` or `TxBuffer`).
pub(crate) struct DescRing<B> {
    /// DMA-coherent memory region holding the hardware descriptors.
    ring: DmaCoherent,
    /// Per-slot buffer metadata.
    buffers: Vec<Option<B>>,
    /// Number of descriptors in the ring (must be a multiple of 8).
    count: u16,
    /// Size of a single descriptor in bytes.
    desc_size: usize,
    /// Index of the next descriptor to be used by software.
    next_to_use: u16,
    /// Index of the next descriptor to be cleaned (reclaimed) by software.
    next_to_clean: u16,
}

impl<B> DescRing<B> {
    /// Returns the number of descriptors in the ring.
    pub fn count(&self) -> u16 {
        self.count
    }

    /// Returns the DMA (bus) address of the ring memory.
    pub fn dma_addr(&self) -> u64 {
        self.ring.daddr() as u64
    }

    /// Returns the total byte size of the descriptor ring.
    pub fn ring_size_bytes(&self) -> u32 {
        (self.count as usize * self.desc_size) as u32
    }

    /// Returns the next-to-use index.
    pub fn next_to_use(&self) -> u16 {
        self.next_to_use
    }

    /// Sets the next-to-use index.
    pub fn set_next_to_use(&mut self, val: u16) {
        debug_assert!(val < self.count);
        self.next_to_use = val;
    }

    /// Returns the next-to-clean index.
    pub fn next_to_clean(&self) -> u16 {
        self.next_to_clean
    }

    /// Sets the next-to-clean index.
    pub fn set_next_to_clean(&mut self, val: u16) {
        debug_assert!(val < self.count);
        self.next_to_clean = val;
    }

    /// Advances a ring index by one, wrapping at `count`.
    pub fn advance_index(&self, idx: u16) -> u16 {
        let next = idx + 1;
        if next >= self.count { 0 } else { next }
    }

    /// Returns the number of unused (available) descriptor slots.
    ///
    /// One slot is always reserved to distinguish full from empty.
    pub fn unused_count(&self) -> u16 {
        if self.next_to_clean > self.next_to_use {
            self.next_to_clean - self.next_to_use - 1
        } else {
            self.count + self.next_to_clean - self.next_to_use - 1
        }
    }

    /// Reads a descriptor of type `D` at the given index.
    pub fn read_desc<D: Pod>(&self, idx: u16) -> D {
        debug_assert!(size_of::<D>() == self.desc_size);
        self.ring
            .read_val::<D>(idx as usize * self.desc_size)
            .unwrap()
    }

    /// Writes a descriptor of type `D` at the given index.
    pub fn write_desc<D: Pod>(&self, idx: u16, desc: &D) {
        debug_assert!(size_of::<D>() == self.desc_size);
        self.ring
            .write_val::<D>(idx as usize * self.desc_size, desc)
            .unwrap();
    }

    /// Returns a reference to the buffer slot at the given index.
    pub fn buffer(&self, idx: u16) -> &Option<B> {
        &self.buffers[idx as usize]
    }

    /// Returns a mutable reference to the buffer slot at the given index.
    pub fn buffer_mut(&mut self, idx: u16) -> &mut Option<B> {
        &mut self.buffers[idx as usize]
    }

    /// Takes the buffer out of the slot at the given index, leaving `None`.
    pub fn take_buffer(&mut self, idx: u16) -> Option<B> {
        self.buffers[idx as usize].take()
    }

    /// Places a buffer into the slot at the given index.
    pub fn put_buffer(&mut self, idx: u16, buf: B) {
        self.buffers[idx as usize] = Some(buf);
    }

    /// Resets the ring indices to zero and zeroes all descriptor memory.
    pub fn reset(&mut self) {
        self.next_to_use = 0;
        self.next_to_clean = 0;
        let zeros = [0u8; 16];
        for i in 0..self.count {
            let offset = i as usize * self.desc_size;
            self.ring
                .write_bytes(offset, &zeros[..self.desc_size])
                .expect("failed to zero descriptor slot");
        }
    }
}

/// RX-specific operations on the descriptor ring.
impl DescRing<RxBuffer> {
    /// Refills a single RX descriptor slot to point at the given buffer.
    ///
    /// Called when replenishing the ring after processing a received packet.
    pub fn refill_desc(&self, idx: u16, rx_buffer: &RxBuffer) {
        let read_desc = RxDescRead {
            buffer_addr: rx_buffer.daddr() as u64,
            reserved: 0,
        };
        self.ring
            .write_val::<RxDescRead>(idx as usize * RX_DESC_EXT_SIZE, &read_desc)
            .unwrap();
    }
}

// =============================================================================
// Ring Allocation
// =============================================================================

/// Error type for ring allocation failures.
#[derive(Debug)]
pub(crate) struct RingAllocError;

/// Computes the number of 4 KiB pages needed for `count` descriptors of `desc_size` bytes.
fn pages_for_ring(count: u16, desc_size: usize) -> usize {
    let total_bytes = count as usize * desc_size;
    total_bytes.div_ceil(4096)
}

/// Allocates a TX descriptor ring.
///
/// Returns a `DescRing<TxBuffer>` with all descriptors zeroed and no buffers
/// attached. The ring's `count` must be a positive multiple of 8 (hardware
/// requirement).
pub(crate) fn alloc_tx_ring(count: u16) -> Result<DescRing<TxBuffer>, RingAllocError> {
    debug_assert!(
        count > 0 && count % 8 == 0,
        "TX ring count must be a positive multiple of 8"
    );

    let nframes = pages_for_ring(count, TX_DESC_SIZE);
    // `DmaCoherent::alloc` zeroes the memory, so descriptors start as all-zero.
    let ring = DmaCoherent::alloc(nframes, false).map_err(|_| RingAllocError)?;

    let buffers = (0..count as usize).map(|_| None).collect();

    Ok(DescRing {
        ring,
        buffers,
        count,
        desc_size: TX_DESC_SIZE,
        next_to_use: 0,
        next_to_clean: 0,
    })
}

/// Allocates an RX descriptor ring and pre-populates it with receive buffers.
///
/// Each slot receives an `RxBuffer` from the provided DMA pool, and the
/// corresponding descriptor's `buffer_addr` field is set to the buffer's DMA
/// address. The ring's `count` must be a positive multiple of 8 (hardware
/// requirement).
pub(crate) fn alloc_rx_ring(
    count: u16,
    rx_pool: &Arc<DmaPool<FromDevice>>,
) -> Result<DescRing<RxBuffer>, RingAllocError> {
    debug_assert!(
        count > 0 && count % 8 == 0,
        "RX ring count must be a positive multiple of 8"
    );

    let nframes = pages_for_ring(count, RX_DESC_EXT_SIZE);
    let ring = DmaCoherent::alloc(nframes, false).map_err(|_| RingAllocError)?;

    let mut buffers = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let rx_buffer = RxBuffer::new(0, rx_pool).map_err(|_| RingAllocError)?;

        // Write the read-format descriptor so hardware knows where to DMA.
        let read_desc = RxDescRead {
            buffer_addr: rx_buffer.daddr() as u64,
            reserved: 0,
        };
        ring.write_val::<RxDescRead>(i * RX_DESC_EXT_SIZE, &read_desc)
            .map_err(|_| RingAllocError)?;

        buffers.push(Some(rx_buffer));
    }

    Ok(DescRing {
        ring,
        buffers,
        count,
        desc_size: RX_DESC_EXT_SIZE,
        next_to_use: 0,
        next_to_clean: 0,
    })
}
