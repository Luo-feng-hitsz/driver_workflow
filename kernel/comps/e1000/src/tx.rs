// SPDX-License-Identifier: MPL-2.0

//! Transmit path for the e1000 82540EM.
//! TX ring setup, configure_tx, xmit_frame, and TX completion (clean_tx_irq).
//! Translated from e1000_main.c TX-related functions.

use alloc::vec::Vec;

use ostd::mm::{
    Daddr, FrameAllocOptions, HasDaddr, VmReader, VmWriter,
    dma::{DmaStream, FromAndToDevice, ToDevice},
    io::util::HasVmReaderWriter,
};

use crate::desc::{DescRing, E1000TxDesc};
use crate::hw::E1000Hw;
use crate::regs::*;

/// Number of TX descriptors in the ring.
pub const TX_DESC_COUNT: usize = 256;

/// Size in bytes of the TX descriptor ring.
const TX_DESC_RING_SIZE: usize = TX_DESC_COUNT * E1000TxDesc::SIZE;

/// Represents a transmit buffer that has been submitted to hardware.
#[allow(dead_code)]
struct TxBufferInfo {
    /// DMA stream backing this TX buffer.
    dma_stream: DmaStream<ToDevice>,
    /// Length of data.
    length: usize,
}

/// TX ring state.
pub struct TxRing {
    /// DMA region holding the descriptor ring (bidirectional since HW writes back DD).
    desc_dma: DmaStream<FromAndToDevice>,
    /// Descriptor ring tracker.
    ring: DescRing<E1000TxDesc>,
    /// Buffer info for each descriptor slot.
    buffers: Vec<Option<TxBufferInfo>>,
    /// The txd_cmd base (RS | IFCS | EOP for legacy descriptors).
    txd_cmd: u8,
}

impl TxRing {
    /// Allocates and returns a new TX ring. Does NOT program hardware registers.
    pub fn new() -> Result<Self, &'static str> {
        // Allocate pages for the descriptor ring (must be 4K-aligned, >=1 page)
        let num_pages = (TX_DESC_RING_SIZE + 4095) / 4096;
        let segment = FrameAllocOptions::new()
            .alloc_segment(num_pages)
            .map_err(|_| "Failed to allocate TX descriptor ring")?;
        let desc_dma =
            DmaStream::<FromAndToDevice>::map(segment.into(), true)
                .map_err(|_| "Failed to map TX descriptor ring for DMA")?;

        let mut buffers = Vec::with_capacity(TX_DESC_COUNT);
        for _ in 0..TX_DESC_COUNT {
            buffers.push(None);
        }

        Ok(Self {
            desc_dma,
            ring: DescRing::new(TX_DESC_COUNT),
            buffers,
            txd_cmd: TXD_CMD_EOP | TXD_CMD_IFCS | TXD_CMD_RS,
        })
    }

    /// Returns the physical (DMA) address of the descriptor ring.
    pub fn dma_addr(&self) -> Daddr {
        self.desc_dma.daddr()
    }

    /// Returns the ring size in bytes.
    pub fn ring_size_bytes(&self) -> usize {
        TX_DESC_RING_SIZE
    }

    /// Returns true if the ring can accept at least one more packet.
    pub fn can_send(&self) -> bool {
        self.ring.unused_count() >= 1
    }

    /// Returns the number of available descriptor slots.
    pub fn available(&self) -> usize {
        self.ring.unused_count()
    }

    /// Transmits a single frame by writing a legacy TX descriptor.
    /// The packet data is copied into a DMA buffer.
    pub fn xmit_frame(&mut self, hw: &E1000Hw, packet: &[u8]) -> Result<(), &'static str> {
        if self.ring.is_full() {
            return Err("TX ring full");
        }

        let idx = self.ring.next_to_use();

        // Allocate a DMA buffer for the packet
        let num_pages = (packet.len() + 4095) / 4096;
        let segment = FrameAllocOptions::new()
            .alloc_segment(num_pages)
            .map_err(|_| "Failed to allocate TX buffer")?;
        let dma_stream =
            DmaStream::<ToDevice>::map(segment.into(), false)
                .map_err(|_| "Failed to map TX buffer for DMA")?;

        // Copy packet data into the DMA buffer
        {
            let mut writer = dma_stream.writer().map_err(|_| "TX DMA writer error")?;
            writer.write(&mut VmReader::from(packet));
        }
        // Sync to device
        dma_stream
            .sync_to_device(0..packet.len())
            .map_err(|_| "TX sync error")?;

        let buf_daddr = dma_stream.daddr();

        // Write the TX descriptor
        let desc = E1000TxDesc::new_data(buf_daddr as u64, packet.len() as u16, self.txd_cmd);
        self.write_desc(idx, &desc);

        // Store buffer info
        self.buffers[idx] = Some(TxBufferInfo {
            dma_stream,
            length: packet.len(),
        });

        // Advance the ring
        self.ring.advance_use();

        // Update the TDT (Transmit Descriptor Tail) to notify hardware
        hw.regs.write(TDT, self.ring.next_to_use() as u32);

        Ok(())
    }

    /// Cleans completed TX descriptors, freeing their DMA buffers.
    /// Returns the number of descriptors cleaned.
    pub fn clean_tx_irq(&mut self) -> usize {
        let mut cleaned = 0;

        loop {
            let idx = self.ring.next_to_clean();
            if idx == self.ring.next_to_use() {
                break;
            }

            let desc = self.read_desc(idx);
            if !desc.done() {
                break;
            }

            // Free the buffer
            self.buffers[idx] = None;
            self.ring.advance_clean();
            cleaned += 1;
        }

        cleaned
    }

    /// Writes a descriptor to the ring at the given index.
    fn write_desc(&self, index: usize, desc: &E1000TxDesc) {
        let offset = index * E1000TxDesc::SIZE;
        let bytes = bytemuck_cast_desc_to_bytes(desc);
        let mut writer = self.desc_dma.writer().unwrap();
        writer.skip(offset);
        writer.write(&mut VmReader::from(bytes.as_slice()));
        // Sync the descriptor to device
        let _ = self.desc_dma.sync_to_device(offset..offset + E1000TxDesc::SIZE);
    }

    /// Reads a descriptor from the ring at the given index.
    fn read_desc(&self, index: usize) -> E1000TxDesc {
        let offset = index * E1000TxDesc::SIZE;
        // Sync from device first
        let _ = self.desc_dma.sync_from_device(offset..offset + E1000TxDesc::SIZE);
        let mut buf = [0u8; 16];
        let mut reader = self.desc_dma.reader().unwrap();
        reader.skip(offset);
        reader.read(&mut VmWriter::from(&mut buf as &mut [u8]));
        bytemuck_cast_bytes_to_desc(&buf)
    }
}

/// Configures the TX hardware registers.
pub fn configure_tx(hw: &E1000Hw, tx_ring: &TxRing) {
    let dma_addr = tx_ring.dma_addr();

    // Program the base address
    hw.regs.write(TDBAL, (dma_addr & 0xFFFF_FFFF) as u32);
    hw.regs.write(TDBAH, ((dma_addr >> 32) & 0xFFFF_FFFF) as u32);

    // Program the length
    hw.regs.write(TDLEN, tx_ring.ring_size_bytes() as u32);

    // Set head and tail to 0
    hw.regs.write(TDH, 0);
    hw.regs.write(TDT, 0);

    // Set TX interrupt delay
    hw.regs.write(TIDV, 8); // 8 * 1.024us = ~8us

    // Set TX descriptor control for writeback
    hw.regs.write(TXDCTL, TXDCTL_FULL_TX_DESC_WB);

    // Configure TCTL: enable TX, pad short packets, set collision params
    let tctl = TCTL_EN
        | TCTL_PSP
        | (COLLISION_THRESHOLD << TCTL_CT_SHIFT)
        | (COLLISION_DISTANCE_FD << TCTL_COLD_SHIFT);
    hw.regs.write(TCTL, tctl);

    // Set TIPG (Inter-Packet Gap) for copper
    let tipg = TIPG_IPGT_COPPER
        | (TIPG_IPGR1 << TIPG_IPGR1_SHIFT)
        | (TIPG_IPGR2 << TIPG_IPGR2_SHIFT);
    hw.regs.write(TIPG, tipg);
}

// ============================================================================
// Descriptor byte conversion helpers
// ============================================================================

fn bytemuck_cast_desc_to_bytes(desc: &E1000TxDesc) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[0..8].copy_from_slice(&desc.buffer_addr.to_le_bytes());
    buf[8..12].copy_from_slice(&desc.lower.to_le_bytes());
    buf[12..16].copy_from_slice(&desc.upper.to_le_bytes());
    buf
}

fn bytemuck_cast_bytes_to_desc(bytes: &[u8; 16]) -> E1000TxDesc {
    E1000TxDesc {
        buffer_addr: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
        lower: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        upper: u32::from_le_bytes(bytes[12..16].try_into().unwrap()),
    }
}
