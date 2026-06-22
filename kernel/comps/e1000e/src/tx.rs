// SPDX-License-Identifier: MPL-2.0

//! Transmit path for the Intel 82574L (e1000e).
//!
//! Handles TX ring register programming (TDBAL/TDBAH/TDLEN/TDH/TDT/TXDCTL/
//! TCTL/TIPG/TIDV/TADV), DMA burst setup, TX interrupt handling
//! (clean_tx_irq), xmit_frame logic.

use alloc::vec::Vec;

use ostd::mm::{
    Daddr, FrameAllocOptions, HasDaddr, VmReader, VmWriter,
    dma::{DmaStream, FromAndToDevice, ToDevice},
    io::util::HasVmReaderWriter,
};

use crate::desc::{TxDesc, TXD_CMD_EOP, TXD_CMD_IFCS, TXD_CMD_RS, TXD_STAT_DD, TX_DESC_SIZE};
use crate::regs::*;

/// Number of TX descriptors in the ring.
pub(crate) const TX_DESC_COUNT: usize = 256;

/// Size in bytes of the TX descriptor ring.
const TX_DESC_RING_SIZE: usize = TX_DESC_COUNT * TX_DESC_SIZE;

/// Represents a transmit buffer that has been submitted to hardware.
struct TxBufferInfo {
    /// DMA stream backing this TX buffer.
    _dma_stream: DmaStream<ToDevice>,
    /// Length of data.
    _length: usize,
}

/// TX ring state.
pub(crate) struct TxRing {
    /// DMA region holding the descriptor ring.
    desc_dma: DmaStream<FromAndToDevice>,
    /// Buffer info for each descriptor slot.
    buffers: Vec<Option<TxBufferInfo>>,
    /// Next descriptor to use for submitting.
    next_to_use: usize,
    /// Next descriptor to clean (reclaim).
    next_to_clean: usize,
    /// Total count.
    count: usize,
    /// The txd_cmd base (RS | IFCS | EOP for legacy descriptors).
    txd_cmd: u8,
}

impl TxRing {
    /// Allocates and returns a new TX ring. Does NOT program hardware registers.
    pub fn new() -> Result<Self, &'static str> {
        let num_pages = TX_DESC_RING_SIZE.div_ceil(4096);
        let segment = FrameAllocOptions::new()
            .alloc_segment(num_pages)
            .map_err(|_| "Failed to allocate TX descriptor ring")?;
        let desc_dma = DmaStream::<FromAndToDevice>::map(segment.into(), true)
            .map_err(|_| "Failed to map TX descriptor ring for DMA")?;

        let buffers = (0..TX_DESC_COUNT).map(|_| None).collect();

        Ok(Self {
            desc_dma,
            buffers,
            next_to_use: 0,
            next_to_clean: 0,
            count: TX_DESC_COUNT,
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
        self.unused_count() >= 1
    }

    /// Returns the number of unused (available) descriptor slots.
    fn unused_count(&self) -> usize {
        if self.next_to_clean > self.next_to_use {
            self.next_to_clean - self.next_to_use - 1
        } else {
            self.count + self.next_to_clean - self.next_to_use - 1
        }
    }

    /// Transmits a single frame by writing a legacy TX descriptor.
    pub fn xmit_frame(&mut self, regs: &E1000eRegs, packet: &[u8]) -> Result<(), &'static str> {
        if !self.can_send() {
            return Err("TX ring full");
        }

        let idx = self.next_to_use;

        // Allocate a DMA buffer for the packet
        let num_pages = packet.len().div_ceil(4096);
        let segment = FrameAllocOptions::new()
            .alloc_segment(num_pages)
            .map_err(|_| "Failed to allocate TX buffer")?;
        let dma_stream = DmaStream::<ToDevice>::map(segment.into(), false)
            .map_err(|_| "Failed to map TX buffer for DMA")?;

        // Copy packet data into the DMA buffer
        {
            let mut writer = dma_stream.writer().map_err(|_| "TX DMA writer error")?;
            writer.write(&mut VmReader::from(packet));
        }
        dma_stream
            .sync_to_device(0..packet.len())
            .map_err(|_| "TX sync error")?;

        let buf_daddr = dma_stream.daddr();

        // Write the TX descriptor
        let desc = TxDesc {
            buffer_addr: buf_daddr as u64,
            length: packet.len() as u16,
            cso: 0,
            cmd: self.txd_cmd,
            status: 0,
            css: 0,
            special: 0,
        };
        self.write_desc(idx, &desc);

        // Store buffer info
        self.buffers[idx] = Some(TxBufferInfo {
            _dma_stream: dma_stream,
            _length: packet.len(),
        });

        // Advance the ring
        self.next_to_use = (idx + 1) % self.count;

        // Update TDT to notify hardware
        regs.write(TDT, self.next_to_use as u32);

        Ok(())
    }

    /// Cleans completed TX descriptors, freeing their DMA buffers.
    pub fn clean_tx_irq(&mut self) {
        loop {
            if self.next_to_clean == self.next_to_use {
                break;
            }

            let desc = self.read_desc(self.next_to_clean);
            if desc.status & TXD_STAT_DD == 0 {
                break;
            }

            // Free the buffer
            self.buffers[self.next_to_clean] = None;
            self.next_to_clean = (self.next_to_clean + 1) % self.count;
        }
    }

    /// Writes a descriptor to the ring at the given index.
    fn write_desc(&self, index: usize, desc: &TxDesc) {
        let offset = index * TX_DESC_SIZE;
        let bytes = tx_desc_to_bytes(desc);
        let mut writer = self.desc_dma.writer().unwrap();
        writer.skip(offset);
        writer.write(&mut VmReader::from(bytes.as_slice()));
        let _ = self.desc_dma.sync_to_device(offset..offset + TX_DESC_SIZE);
    }

    /// Reads a descriptor from the ring at the given index.
    fn read_desc(&self, index: usize) -> TxDesc {
        let offset = index * TX_DESC_SIZE;
        let _ = self.desc_dma.sync_from_device(offset..offset + TX_DESC_SIZE);
        let mut buf = [0u8; 16];
        let mut reader = self.desc_dma.reader().unwrap();
        reader.skip(offset);
        reader.read(&mut VmWriter::from(&mut buf as &mut [u8]));
        bytes_to_tx_desc(&buf)
    }
}

/// Configures the TX hardware registers for the 82574L.
pub(crate) fn configure_tx(regs: &E1000eRegs, tx_ring: &TxRing) {
    let dma_addr = tx_ring.dma_addr();

    // Program the base address
    regs.write(TDBAL, (dma_addr & 0xFFFF_FFFF) as u32);
    regs.write(TDBAH, ((dma_addr >> 32) & 0xFFFF_FFFF) as u32);

    // Program the length
    regs.write(TDLEN, tx_ring.ring_size_bytes() as u32);

    // Set head and tail to 0
    regs.write(TDH, 0);
    regs.write(TDT, 0);

    // Set TX interrupt delay
    regs.write(TIDV, 8);

    // Set TX descriptor control for writeback
    regs.write(TXDCTL, TXDCTL_FULL_TX_DESC_WB);

    // Configure TCTL: enable TX, pad short packets, set collision params
    let tctl = TCTL_EN
        | TCTL_PSP
        | (COLLISION_THRESHOLD << TCTL_CT_SHIFT)
        | (COLLISION_DISTANCE_FD << TCTL_COLD_SHIFT);
    regs.write(TCTL, tctl);

    // Set TIPG (Inter-Packet Gap) for copper
    let tipg = TIPG_IPGT_COPPER
        | (TIPG_IPGR1 << TIPG_IPGR1_SHIFT)
        | (TIPG_IPGR2 << TIPG_IPGR2_SHIFT);
    regs.write(TIPG, tipg);
}

// ============================================================================
// Descriptor byte conversion helpers
// ============================================================================

fn tx_desc_to_bytes(desc: &TxDesc) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[0..8].copy_from_slice(&desc.buffer_addr.to_le_bytes());
    buf[8..10].copy_from_slice(&desc.length.to_le_bytes());
    buf[10] = desc.cso;
    buf[11] = desc.cmd;
    buf[12] = desc.status;
    buf[13] = desc.css;
    buf[14..16].copy_from_slice(&desc.special.to_le_bytes());
    buf
}

fn bytes_to_tx_desc(bytes: &[u8; 16]) -> TxDesc {
    TxDesc {
        buffer_addr: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
        length: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
        cso: bytes[10],
        cmd: bytes[11],
        status: bytes[12],
        css: bytes[13],
        special: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
    }
}
