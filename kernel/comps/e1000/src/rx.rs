// SPDX-License-Identifier: MPL-2.0

//! Receive path for the e1000 82540EM.
//! RX ring setup, configure_rx, buffer allocation, clean_rx_irq.
//! Translated from e1000_main.c RX-related functions.

use alloc::sync::Arc;
use alloc::vec::Vec;

use aster_network::RxBuffer;
use aster_network::dma_pool::DmaPool;
use ostd::mm::{
    Daddr, HasDaddr, VmReader, VmWriter,
    dma::{DmaStream, FromAndToDevice, FromDevice},
    io::util::HasVmReaderWriter,
    FrameAllocOptions,
};

use crate::desc::{DescRing, E1000RxDesc};
use crate::hw::E1000Hw;
use crate::regs::*;

/// Number of RX descriptors in the ring.
pub const RX_DESC_COUNT: usize = 256;

/// Size in bytes of the RX descriptor ring.
const RX_DESC_RING_SIZE: usize = RX_DESC_COUNT * E1000RxDesc::SIZE;

/// Default receive buffer size (2048 bytes, enough for standard Ethernet frames).
pub const RX_BUFFER_SIZE: usize = 2048;

/// Represents an RX buffer associated with a descriptor slot.
struct RxBufferInfo {
    /// The RxBuffer from the DMA pool.
    rx_buffer: RxBuffer,
}

/// RX ring state.
pub struct RxRing {
    /// DMA-coherent region holding the descriptor ring.
    desc_dma: DmaStream<FromAndToDevice>,
    /// Descriptor ring tracker.
    ring: DescRing<E1000RxDesc>,
    /// Buffer info for each descriptor slot.
    buffers: Vec<Option<RxBufferInfo>>,
    /// DMA pool for RX buffers.
    rx_pool: Arc<DmaPool<FromDevice>>,
}

impl RxRing {
    /// Allocates and returns a new RX ring.
    pub fn new(rx_pool: Arc<DmaPool<FromDevice>>) -> Result<Self, &'static str> {
        // Allocate pages for the descriptor ring
        let num_pages = (RX_DESC_RING_SIZE + 4095) / 4096;
        let segment = FrameAllocOptions::new()
            .alloc_segment(num_pages)
            .map_err(|_| "Failed to allocate RX descriptor ring")?;
        let desc_dma =
            DmaStream::<FromAndToDevice>::map(segment.into(), true)
                .map_err(|_| "Failed to map RX descriptor ring for DMA")?;

        let mut buffers = Vec::with_capacity(RX_DESC_COUNT);
        for _ in 0..RX_DESC_COUNT {
            buffers.push(None);
        }

        Ok(Self {
            desc_dma,
            ring: DescRing::new(RX_DESC_COUNT),
            buffers,
            rx_pool,
        })
    }

    /// Returns the physical (DMA) address of the descriptor ring.
    pub fn dma_addr(&self) -> Daddr {
        self.desc_dma.daddr()
    }

    /// Returns the ring size in bytes.
    pub fn ring_size_bytes(&self) -> usize {
        RX_DESC_RING_SIZE
    }

    /// Allocates RX buffers and fills the descriptor ring.
    /// Should be called during initialization.
    pub fn alloc_rx_buffers(&mut self, hw: &E1000Hw) -> Result<(), &'static str> {
        for i in 0..RX_DESC_COUNT {
            self.alloc_single_buffer(i)?;
        }
        // Set next_to_use to count-1 so hardware sees all descriptors available
        self.ring.set_next_to_use(RX_DESC_COUNT - 1);
        // Write RDT to notify hardware
        hw.regs.write(RDT, (RX_DESC_COUNT - 1) as u32);
        Ok(())
    }

    /// Allocates a single RX buffer for the given descriptor index.
    fn alloc_single_buffer(&mut self, index: usize) -> Result<(), &'static str> {
        // Allocate from the DMA pool (header_len = 0 for e1000 legacy descriptors)
        let rx_buffer =
            RxBuffer::new(0, &self.rx_pool).map_err(|_| "Failed to allocate RX buffer")?;

        // Get the DMA address of the buffer
        let daddr = rx_buffer.daddr();

        // Write the descriptor
        let desc = E1000RxDesc {
            buffer_addr: daddr as u64,
            length: 0,
            csum: 0,
            status: 0,
            errors: 0,
            special: 0,
        };
        self.write_desc(index, &desc);

        self.buffers[index] = Some(RxBufferInfo { rx_buffer });
        Ok(())
    }

    /// Returns true if there are completed RX descriptors ready to process.
    pub fn can_receive(&self) -> bool {
        let idx = self.ring.next_to_clean();
        let desc = self.read_desc(idx);
        desc.done()
    }

    /// Processes completed RX descriptors, returning received packets.
    /// This is the equivalent of e1000_clean_rx_irq.
    pub fn clean_rx_irq(&mut self, hw: &E1000Hw) -> Option<RxBuffer> {
        let idx = self.ring.next_to_clean();
        let desc = self.read_desc(idx);

        if !desc.done() {
            return None;
        }

        // Check for errors
        if desc.has_error() {
            // Recycle the buffer
            self.recycle_buffer(idx, hw);
            return None;
        }

        // Take the completed buffer
        let buf_info = self.buffers[idx].take()?;
        let mut rx_buffer = buf_info.rx_buffer;

        // Set payload length (the hardware wrote the length)
        let pkt_len = desc.length as usize;
        rx_buffer.set_payload_len(pkt_len);

        // Advance clean pointer
        self.ring.advance_clean();

        // Allocate a new buffer for this slot and update RDT
        let _ = self.alloc_single_buffer(idx);
        self.ring.set_next_to_use(idx);
        hw.regs.write(RDT, idx as u32);

        Some(rx_buffer)
    }

    /// Recycles a buffer (on error), keeping it in place and resetting the descriptor.
    fn recycle_buffer(&mut self, index: usize, hw: &E1000Hw) {
        // Re-read the buffer DMA address from the existing buffer
        if let Some(ref buf_info) = self.buffers[index] {
            let daddr = buf_info.rx_buffer.daddr();
            let desc = E1000RxDesc {
                buffer_addr: daddr as u64,
                length: 0,
                csum: 0,
                status: 0,
                errors: 0,
                special: 0,
            };
            self.write_desc(index, &desc);
        }
        self.ring.advance_clean();
        hw.regs.write(RDT, index as u32);
    }

    /// Writes a descriptor to the ring at the given index.
    fn write_desc(&self, index: usize, desc: &E1000RxDesc) {
        let offset = index * E1000RxDesc::SIZE;
        let bytes = rx_desc_to_bytes(desc);
        let mut writer = self.desc_dma.writer().unwrap();
        writer.skip(offset);
        writer.write(&mut VmReader::from(bytes.as_slice()));
        let _ = self.desc_dma.sync_to_device(offset..offset + E1000RxDesc::SIZE);
    }

    /// Reads a descriptor from the ring at the given index.
    fn read_desc(&self, index: usize) -> E1000RxDesc {
        let offset = index * E1000RxDesc::SIZE;
        let _ = self.desc_dma.sync_from_device(offset..offset + E1000RxDesc::SIZE);
        let mut buf = [0u8; 16];
        let mut reader = self.desc_dma.reader().unwrap();
        reader.skip(offset);
        reader.read(&mut VmWriter::from(&mut buf as &mut [u8]));
        bytes_to_rx_desc(&buf)
    }
}

/// Configures the RX hardware registers.
pub fn configure_rx(hw: &E1000Hw, rx_ring: &RxRing) {
    let dma_addr = rx_ring.dma_addr();

    // Program the base address
    hw.regs.write(RDBAL, (dma_addr & 0xFFFF_FFFF) as u32);
    hw.regs.write(RDBAH, ((dma_addr >> 32) & 0xFFFF_FFFF) as u32);

    // Program the length
    hw.regs.write(RDLEN, rx_ring.ring_size_bytes() as u32);

    // Set head and tail
    hw.regs.write(RDH, 0);
    hw.regs.write(RDT, 0);

    // Set RX delay timer
    hw.regs.write(RDTR, 0); // No delay

    // Configure RCTL
    let rctl = RCTL_EN
        | RCTL_BAM         // Accept broadcast
        | RCTL_SZ_2048     // Buffer size 2048
        | RCTL_SECRC       // Strip CRC
        | RCTL_LBM_NO      // No loopback
        | RCTL_RDMTS_HALF; // RX desc min threshold = half
    hw.regs.write(RCTL, rctl);

    // Enable RX checksum offload
    let rxcsum = RXCSUM_IPOFL | RXCSUM_TUOFL;
    hw.regs.write(RXCSUM, rxcsum);
}

// ============================================================================
// Descriptor byte conversion helpers
// ============================================================================

fn rx_desc_to_bytes(desc: &E1000RxDesc) -> [u8; 16] {
    let mut buf = [0u8; 16];
    buf[0..8].copy_from_slice(&desc.buffer_addr.to_le_bytes());
    buf[8..10].copy_from_slice(&desc.length.to_le_bytes());
    buf[10..12].copy_from_slice(&desc.csum.to_le_bytes());
    buf[12] = desc.status;
    buf[13] = desc.errors;
    buf[14..16].copy_from_slice(&desc.special.to_le_bytes());
    buf
}

fn bytes_to_rx_desc(bytes: &[u8; 16]) -> E1000RxDesc {
    E1000RxDesc {
        buffer_addr: u64::from_le_bytes(bytes[0..8].try_into().unwrap()),
        length: u16::from_le_bytes(bytes[8..10].try_into().unwrap()),
        csum: u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
        status: bytes[12],
        errors: bytes[13],
        special: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
    }
}
