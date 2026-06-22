// SPDX-License-Identifier: MPL-2.0

//! Receive path for the Intel 82574L (e1000e).
//!
//! Handles:
//! - RX ring register programming (RDBAL/RDBAH/RDLEN/RDH/RDT/RXDCTL/RCTL/RDTR/RADV)
//! - RX buffer allocation using extended descriptors
//! - RX interrupt handling (`clean_rx_irq`)
//! - Receive checksum verification
//! - Packet delivery via `RxBuffer`
//!
//! Only the 82574L code path is implemented. Packet-split (PS) and jumbo
//! frame paths are intentionally omitted because we target standard MTU
//! (1500 bytes) with extended RX descriptors.

use alloc::sync::Arc;

use aster_network::{RxBuffer, dma_pool::DmaPool};
use ostd::{io::IoMem, mm::VmIoOnce, mm::dma::FromDevice};

use crate::desc::{
    DescRing, RxDescExt, RXDEXT_ERR_FRAME_MASK, RXDEXT_STAT_DD, RXDEXT_STAT_EOP,
    RXDEXT_STAT_IXSM, RXDEXT_STAT_TCPCS, RXDEXT_STAT_UDPCS,
};

// =============================================================================
// Register Offsets (MMIO, queue 0 only -- 82574L has a single RX queue)
// =============================================================================

/// Receive Control register.
const REG_RCTL: usize = 0x00100;
/// Receive Descriptor Base Address Low (queue 0).
const REG_RDBAL: usize = 0x02800;
/// Receive Descriptor Base Address High (queue 0).
const REG_RDBAH: usize = 0x02804;
/// Receive Descriptor Length (queue 0).
const REG_RDLEN: usize = 0x02808;
/// Receive Descriptor Head (queue 0).
const REG_RDH: usize = 0x02810;
/// Receive Descriptor Tail (queue 0).
const REG_RDT: usize = 0x02818;
/// Receive Delay Timer.
const REG_RDTR: usize = 0x02820;
/// Receive Descriptor Control (queue 0).
const REG_RXDCTL: usize = 0x02828;
/// Receive Interrupt Absolute Delay Timer.
const REG_RADV: usize = 0x0282C;
/// Receive Checksum Control.
const REG_RXCSUM: usize = 0x05000;
/// Receive Filter Control.
const REG_RFCTL: usize = 0x05008;
/// Extended Device Control.
const REG_CTRL_EXT: usize = 0x00018;
/// Interrupt Acknowledge Auto Mask.
const REG_IAM: usize = 0x000E0;

// =============================================================================
// RCTL Bits
// =============================================================================

const RCTL_EN: u32 = 1 << 1;
const RCTL_SBP: u32 = 1 << 2;
const RCTL_BAM: u32 = 1 << 15;
const RCTL_LBM_NO: u32 = 0;
const RCTL_RDMTS_HALF: u32 = 0;
const RCTL_MO_SHIFT: u32 = 12;
const RCTL_LPE: u32 = 1 << 5;
const RCTL_SECRC: u32 = 1 << 26;
const RCTL_BSEX: u32 = 1 << 25;
const RCTL_SZ_2048: u32 = 0 << 16;
const RCTL_SZ_4096: u32 = 3 << 16;

// =============================================================================
// RFCTL Bits
// =============================================================================

/// Enable extended status in receive descriptors.
const RFCTL_EXTEN: u32 = 0x0000_8000;

// =============================================================================
// RXCSUM Bits
// =============================================================================

/// TCP/UDP checksum offload enable.
const RXCSUM_TUOFL: u32 = 0x0000_0200;

// =============================================================================
// CTRL_EXT Bits
// =============================================================================

/// Interrupt Acknowledge Auto-Mask.
const CTRL_EXT_IAME: u32 = 0x0800_0000;

// =============================================================================
// RXDCTL DMA Burst Configuration (82574L)
// =============================================================================

/// RXDCTL value for DMA burst mode:
///   - granularity = 1 (descriptor granularity)
///   - wthresh = 4
///   - hthresh = 4
///   - pthresh = 0x20
const RXDCTL_DMA_BURST_ENABLE: u32 = 0x0100_0000 | (4 << 16) | (4 << 8) | 0x20;

// =============================================================================
// Misc Constants
// =============================================================================

/// Batching threshold: replenish hardware after this many descriptors are cleaned.
/// Must be a power of 2.
const RX_BUFFER_WRITE: u16 = 16;

/// Ethernet CRC length in bytes.
const ETH_FCS_LEN: u16 = 4;

/// Checksum error bits in the upper byte of status_error (bits [31:24]).
/// TCP/UDP checksum error.
const RXD_ERR_TCPE: u32 = 1 << 29;
/// IP checksum error.
const RXD_ERR_IPE: u32 = 1 << 30;

// =============================================================================
// Checksum Result
// =============================================================================

/// Outcome of hardware receive checksum verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RxChecksumResult {
    /// Checksum was not computed by hardware (caller must verify in software).
    None,
    /// Hardware detected a checksum error.
    Error,
    /// Hardware verified the checksum is correct -- no software verification needed.
    Unnecessary,
}

/// Checks the hardware checksum offload result from the extended RX descriptor.
///
/// Corresponds to `e1000_rx_checksum()` in the Linux driver.
///
/// The 82574L computes TCP/UDP checksums and reports the result in the
/// `status_error` field of the extended RX descriptor writeback format.
///
/// If `rx_csum_enabled` is false, returns `None` (hardware offload disabled).
pub(crate) fn rx_checksum(status_error: u32, rx_csum_enabled: bool) -> RxChecksumResult {
    if !rx_csum_enabled {
        return RxChecksumResult::None;
    }

    // Ignore Checksum Indication -- hardware did not compute checksums.
    if status_error & RXDEXT_STAT_IXSM != 0 {
        return RxChecksumResult::None;
    }

    // Check TCP/UDP or IP checksum error bits (in the error byte, bits [31:24]).
    if status_error & (RXD_ERR_TCPE | RXD_ERR_IPE) != 0 {
        return RxChecksumResult::Error;
    }

    // If neither TCP nor UDP checksum was calculated, nothing to report.
    if status_error & (RXDEXT_STAT_TCPCS | RXDEXT_STAT_UDPCS) == 0 {
        return RxChecksumResult::None;
    }

    // Valid TCP or UDP checksum computed and no errors.
    RxChecksumResult::Unnecessary
}

// =============================================================================
// RX Ring Wrapper
// =============================================================================

/// Configuration for RX interrupt coalescing.
pub(crate) struct RxCoalesceConfig {
    /// Receive interrupt delay in microseconds (RDTR).
    pub rx_int_delay: u32,
    /// Receive interrupt absolute delay in microseconds (RADV).
    pub rx_abs_int_delay: u32,
}

impl Default for RxCoalesceConfig {
    fn default() -> Self {
        Self {
            // Conservative defaults matching Linux e1000e defaults for 82574L.
            rx_int_delay: 0,
            rx_abs_int_delay: 8,
        }
    }
}

/// Manages the RX descriptor ring and associated hardware state.
pub(crate) struct RxRing {
    /// The underlying descriptor ring.
    ring: DescRing<RxBuffer>,
    /// DMA pool from which RX buffers are allocated.
    rx_pool: Arc<DmaPool<FromDevice>>,
    /// Whether hardware RX checksum offload is enabled.
    rx_csum_enabled: bool,
    /// Whether CRC stripping is enabled (FLAG2_CRC_STRIPPING).
    crc_stripping: bool,
    /// Whether DMA burst mode is enabled (FLAG2_DMA_BURST).
    dma_burst: bool,
    /// Whether we are currently discarding a multi-descriptor frame.
    is_discarding: bool,
}

impl RxRing {
    /// Creates a new `RxRing` from a pre-allocated descriptor ring and DMA pool.
    pub fn new(
        ring: DescRing<RxBuffer>,
        rx_pool: Arc<DmaPool<FromDevice>>,
        rx_csum_enabled: bool,
        crc_stripping: bool,
        dma_burst: bool,
    ) -> Self {
        Self {
            ring,
            rx_pool,
            rx_csum_enabled,
            crc_stripping,
            dma_burst,
            is_discarding: false,
        }
    }

    /// Returns the DMA address of the descriptor ring.
    pub fn dma_addr(&self) -> u64 {
        self.ring.dma_addr()
    }

    /// Returns the total byte length of the descriptor ring.
    pub fn ring_len_bytes(&self) -> u32 {
        self.ring.ring_size_bytes()
    }

    /// Returns the number of descriptors.
    pub fn count(&self) -> u16 {
        self.ring.count()
    }

    /// Returns true if there may be received packets to process.
    ///
    /// Peeks at the next-to-clean descriptor's DD bit without modifying state.
    pub fn can_receive(&self) -> bool {
        let idx = self.ring.next_to_clean();
        let desc: RxDescExt = self.ring.read_desc(idx);
        desc.status_error & RXDEXT_STAT_DD != 0
    }

    // -------------------------------------------------------------------------
    // Hardware Configuration
    // -------------------------------------------------------------------------

    /// Programs the RX hardware registers after a reset.
    ///
    /// Corresponds to `e1000_configure_rx()` in the Linux driver, 82574L path only.
    /// The caller is responsible for calling `e1000_setup_rctl()` before this.
    pub fn configure_rx(&self, io_mem: &IoMem, coalesce: &RxCoalesceConfig) {
        let rdba = self.ring.dma_addr();
        let rdlen = self.ring.ring_size_bytes();

        // Disable receiver while we reconfigure.
        let rctl = read_reg(io_mem, REG_RCTL);
        write_reg(io_mem, REG_RCTL, rctl & !RCTL_EN);
        // Flush posted writes.
        let _ = read_reg(io_mem, REG_RCTL);

        // DMA burst configuration for 82574L (FLAG2_DMA_BURST).
        if self.dma_burst {
            write_reg(io_mem, REG_RXDCTL, RXDCTL_DMA_BURST_ENABLE);
        }

        // Receive delay timer and absolute delay timer for interrupt coalescing.
        write_reg(io_mem, REG_RDTR, coalesce.rx_int_delay);
        write_reg(io_mem, REG_RADV, coalesce.rx_abs_int_delay);

        // Auto-Mask interrupts upon ICR access.
        let ctrl_ext = read_reg(io_mem, REG_CTRL_EXT);
        write_reg(io_mem, REG_CTRL_EXT, ctrl_ext | CTRL_EXT_IAME);
        write_reg(io_mem, REG_IAM, 0xFFFF_FFFF);
        // Flush posted writes.
        let _ = read_reg(io_mem, REG_CTRL_EXT);

        // Program the descriptor ring base, length, and head/tail pointers.
        write_reg(io_mem, REG_RDBAL, rdba as u32);
        write_reg(io_mem, REG_RDBAH, (rdba >> 32) as u32);
        write_reg(io_mem, REG_RDLEN, rdlen);
        write_reg(io_mem, REG_RDH, 0);
        write_reg(io_mem, REG_RDT, 0);

        // Enable TCP/UDP checksum offload.
        let rxcsum = read_reg(io_mem, REG_RXCSUM);
        if self.rx_csum_enabled {
            write_reg(io_mem, REG_RXCSUM, rxcsum | RXCSUM_TUOFL);
        } else {
            write_reg(io_mem, REG_RXCSUM, rxcsum & !RXCSUM_TUOFL);
        }

        // Re-enable receiver.
        write_reg(io_mem, REG_RCTL, rctl);
    }

    /// Programs the RCTL register for standard (non-jumbo, non-PS) operation.
    ///
    /// Corresponds to `e1000_setup_rctl()` in the Linux driver, 82574L path only.
    /// Does NOT enable packet-split or jumbo frame paths.
    pub fn setup_rctl(&self, io_mem: &IoMem, mc_filter_type: u32) {
        let mut rctl = read_reg(io_mem, REG_RCTL);

        // Clear multicast offset field and rewrite.
        rctl &= !(3 << RCTL_MO_SHIFT);
        rctl |= RCTL_EN
            | RCTL_BAM
            | RCTL_LBM_NO
            | RCTL_RDMTS_HALF
            | (mc_filter_type << RCTL_MO_SHIFT);

        // Do not store bad packets.
        rctl &= !RCTL_SBP;

        // Standard MTU -- no long packet enable.
        rctl &= !RCTL_LPE;

        // Strip Ethernet CRC if enabled.
        if self.crc_stripping {
            rctl |= RCTL_SECRC;
        }

        // Buffer size: 2048 bytes (standard MTU, no BSEX needed).
        rctl &= !RCTL_SZ_4096;
        rctl &= !RCTL_BSEX;
        rctl |= RCTL_SZ_2048;

        // Enable extended status in all receive descriptors.
        let rfctl = read_reg(io_mem, REG_RFCTL);
        write_reg(io_mem, REG_RFCTL, rfctl | RFCTL_EXTEN);

        write_reg(io_mem, REG_RCTL, rctl);
    }

    // -------------------------------------------------------------------------
    // Buffer Allocation
    // -------------------------------------------------------------------------

    /// Refills up to `count` empty RX descriptor slots with fresh buffers.
    ///
    /// Corresponds to `e1000_alloc_rx_buffers()` in the Linux driver.
    /// Updates the hardware tail pointer (RDT) in batches of `RX_BUFFER_WRITE`
    /// to amortize MMIO write overhead.
    pub fn alloc_rx_buffers(&mut self, io_mem: &IoMem, mut count: u16) {
        let mut i = self.ring.next_to_use();

        while count > 0 {
            // Allocate a new RX buffer from the DMA pool.
            let rx_buffer = match RxBuffer::new(0, &self.rx_pool) {
                Ok(buf) => buf,
                Err(_) => break, // Allocation failure; try again later.
            };

            // Write the read-format descriptor (buffer_addr) into the ring.
            self.ring.refill_desc(i, &rx_buffer);
            self.ring.put_buffer(i, rx_buffer);

            // Notify hardware in batches: when `i` is aligned to RX_BUFFER_WRITE.
            if (i & (RX_BUFFER_WRITE - 1)) == 0 {
                write_reg(io_mem, REG_RDT, i as u32);
            }

            i = self.ring.advance_index(i);
            count -= 1;
        }

        self.ring.set_next_to_use(i);
    }

    /// Replenishes all unused descriptor slots.
    pub fn alloc_all_rx_buffers(&mut self, io_mem: &IoMem) {
        let unused = self.ring.unused_count();
        if unused > 0 {
            self.alloc_rx_buffers(io_mem, unused);
        }
    }

    // -------------------------------------------------------------------------
    // RX IRQ Cleaning
    // -------------------------------------------------------------------------

    /// Processes received packets from the RX ring up to `budget`.
    ///
    /// Returns a vector of `RxBuffer` containing the received packet payloads.
    /// This corresponds to `e1000_clean_rx_irq()` in the Linux driver.
    ///
    /// The caller should subsequently call `alloc_rx_buffers` (or
    /// `alloc_all_rx_buffers`) to refill consumed descriptor slots and then
    /// raise the RX softirq.
    pub fn clean_rx_irq(
        &mut self,
        io_mem: &IoMem,
        budget: u32,
    ) -> alloc::vec::Vec<RxBuffer> {
        let mut received = alloc::vec::Vec::new();
        let mut work_done: u32 = 0;
        let mut cleaned_count: u16 = 0;

        let mut i = self.ring.next_to_clean();

        loop {
            if work_done >= budget {
                break;
            }

            let desc: RxDescExt = self.ring.read_desc(i);
            let staterr = desc.status_error;

            // Descriptor not yet written back by hardware.
            if staterr & RXDEXT_STAT_DD == 0 {
                break;
            }

            work_done += 1;
            cleaned_count += 1;

            // Take the buffer out of the ring slot.
            let rx_buffer = match self.ring.take_buffer(i) {
                Some(buf) => buf,
                None => {
                    // Should not happen in a correctly managed ring.
                    i = self.ring.advance_index(i);
                    continue;
                }
            };

            let length = desc.length;

            // Multi-descriptor frames: the 82574L should never produce these
            // with standard MTU and 2048-byte buffers, but handle gracefully.
            if staterr & RXDEXT_STAT_EOP == 0 {
                self.is_discarding = true;
            }

            if self.is_discarding {
                // Drop this buffer; wait for EOP to stop discarding.
                if staterr & RXDEXT_STAT_EOP != 0 {
                    self.is_discarding = false;
                }
                // Advance without delivering the packet.
                i = self.ring.advance_index(i);

                // Batch replenishment.
                if cleaned_count >= RX_BUFFER_WRITE {
                    self.alloc_rx_buffers(io_mem, cleaned_count);
                    cleaned_count = 0;
                }
                continue;
            }

            // Check for frame-level errors (CRC, symbol, sequence, etc.).
            if staterr & RXDEXT_ERR_FRAME_MASK != 0 {
                // Drop the errored frame.
                i = self.ring.advance_index(i);

                if cleaned_count >= RX_BUFFER_WRITE {
                    self.alloc_rx_buffers(io_mem, cleaned_count);
                    cleaned_count = 0;
                }
                continue;
            }

            // Adjust length: strip CRC if the hardware did not already do so.
            let payload_len = if self.crc_stripping {
                length as usize
            } else {
                (length.saturating_sub(ETH_FCS_LEN)) as usize
            };

            // Set the actual payload length on the RxBuffer so the upper
            // layers know how many bytes are valid.
            let mut rx_buffer = rx_buffer;
            rx_buffer.set_payload_len(payload_len);

            // Optionally verify checksum (informational; we deliver regardless).
            let _csum = rx_checksum(staterr, self.rx_csum_enabled);

            received.push(rx_buffer);

            // Advance to next descriptor.
            i = self.ring.advance_index(i);

            // Batch replenishment.
            if cleaned_count >= RX_BUFFER_WRITE {
                self.alloc_rx_buffers(io_mem, cleaned_count);
                cleaned_count = 0;
            }
        }

        self.ring.set_next_to_clean(i);

        // Final replenishment of any remaining cleaned slots.
        let unused = self.ring.unused_count();
        if unused > 0 {
            self.alloc_rx_buffers(io_mem, unused);
        }

        received
    }
}

// =============================================================================
// MMIO Helpers (private to this module)
// =============================================================================

/// Reads a 32-bit MMIO register.
#[inline]
fn read_reg(io_mem: &IoMem, offset: usize) -> u32 {
    io_mem.read_once(offset).unwrap()
}

/// Writes a 32-bit MMIO register.
#[inline]
fn write_reg(io_mem: &IoMem, offset: usize, value: u32) {
    io_mem.write_once(offset, &value).unwrap();
}
