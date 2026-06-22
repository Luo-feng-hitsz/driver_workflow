// SPDX-License-Identifier: MPL-2.0

//! TX/RX descriptor ring structures for the Intel 82540EM.
//!
//! Descriptor rings are backed by `DmaCoherent` memory. Packet data buffers
//! are allocated from `DmaPool` via `RxBuffer`/`TxBuffer` from aster-network.

use alloc::{sync::Arc, vec::Vec};

use aster_network::{RxBuffer, TxBuffer, dma_pool::DmaPool};
use ostd::mm::{
    HasDaddr, VmIo,
    dma::{DmaCoherent, FromDevice},
};

use crate::regs::*;

// =============================================================================
// Descriptor Structures (repr(C), 16 bytes each)
// =============================================================================

/// Legacy Receive Descriptor (16 bytes).
///
/// Hardware writes `length`, `checksum`, `status`, `errors`, `special`
/// after DMA-ing data into `buffer_addr`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct RxDesc {
    pub buffer_addr: u64,
    pub length: u16,
    pub checksum: u16,
    pub status: u8,
    pub errors: u8,
    pub special: u16,
}

/// Legacy Transmit Descriptor (16 bytes).
///
/// Software fills `buffer_addr`, `length`, `cmd`; hardware writes back `status`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct TxDesc {
    pub buffer_addr: u64,
    pub length: u16,
    pub cso: u8,
    pub cmd: u8,
    pub status: u8,
    pub css: u8,
    pub special: u16,
}

// Compile-time size checks.
const _: () = assert!(size_of::<RxDesc>() == DESC_SIZE);
const _: () = assert!(size_of::<TxDesc>() == DESC_SIZE);

// =============================================================================
// Descriptor ring helper functions
// =============================================================================

/// Reads one RX descriptor from a DMA-coherent ring at the given index.
pub fn read_rx_desc(ring: &DmaCoherent, idx: u16) -> RxDesc {
    ring.read_val::<RxDesc>(idx as usize * DESC_SIZE).unwrap()
}

/// Writes one RX descriptor into a DMA-coherent ring at the given index.
pub fn write_rx_desc(ring: &DmaCoherent, idx: u16, desc: &RxDesc) {
    ring.write_val::<RxDesc>(idx as usize * DESC_SIZE, desc)
        .unwrap();
}

/// Reads one TX descriptor from a DMA-coherent ring at the given index.
pub fn read_tx_desc(ring: &DmaCoherent, idx: u16) -> TxDesc {
    ring.read_val::<TxDesc>(idx as usize * DESC_SIZE).unwrap()
}

/// Writes one TX descriptor into a DMA-coherent ring at the given index.
pub fn write_tx_desc(ring: &DmaCoherent, idx: u16, desc: &TxDesc) {
    ring.write_val::<TxDesc>(idx as usize * DESC_SIZE, desc)
        .unwrap();
}

// =============================================================================
// Ring allocation
// =============================================================================

/// Allocates the RX descriptor ring (DMA-coherent) and initial RX buffers.
///
/// Returns the ring, the buffer vector, and the initial tail index.
pub fn alloc_rx_ring(
    rx_pool: &Arc<DmaPool<FromDevice>>,
) -> Result<(DmaCoherent, Vec<Option<RxBuffer>>), RingAllocError> {
    let num = NUM_RX_DESCS as usize;

    // Each descriptor is 16 bytes; 64 descriptors = 1024 bytes < 1 page.
    let rx_ring = DmaCoherent::alloc(1, false).map_err(|_| RingAllocError)?;

    let mut rx_buffers = Vec::with_capacity(num);
    for i in 0..num {
        let rx_buffer = RxBuffer::new(0, rx_pool).map_err(|_| RingAllocError)?;

        let desc = RxDesc {
            buffer_addr: rx_buffer.daddr() as u64,
            ..RxDesc::default()
        };
        write_rx_desc(&rx_ring, i as u16, &desc);

        rx_buffers.push(Some(rx_buffer));
    }

    Ok((rx_ring, rx_buffers))
}

/// Allocates the TX descriptor ring (DMA-coherent) and initial (empty) buffer slots.
///
/// Returns the ring and the buffer vector.
pub fn alloc_tx_ring() -> Result<(DmaCoherent, Vec<Option<TxBuffer>>), RingAllocError> {
    let num = NUM_TX_DESCS as usize;

    let tx_ring = DmaCoherent::alloc(1, false).map_err(|_| RingAllocError)?;

    // Zero out all TX descriptors
    for i in 0..num {
        write_tx_desc(&tx_ring, i as u16, &TxDesc::default());
    }

    let tx_buffers = (0..num).map(|_| None).collect();
    Ok((tx_ring, tx_buffers))
}

/// Error type for ring allocation failures.
#[derive(Debug)]
pub struct RingAllocError;
