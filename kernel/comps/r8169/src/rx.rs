// SPDX-License-Identifier: MPL-2.0

//! Receive path for the RTL8168g (r8169) network controller.
//!
//! This module implements RX buffer allocation, RX ring fill/clear,
//! RX checksum validation, VLAN tag extraction, fragmented frame
//! detection, descriptor recycling (`mark_to_asic`), and the main
//! `rtl_rx` receive polling loop.
//!
//! Translated from: drivers/net/ethernet/realtek/r8169_main.c

use alloc::sync::Arc;

use aster_network::NetError;
use ostd::mm::{
    Daddr, FrameAllocOptions, HasDaddr,
    dma::{DmaStream, FromDevice},
    io::util::HasVmReaderWriter,
};

use crate::desc::{DescRing, RawDesc, mark_to_asic, is_fragmented_frame};
use crate::regs::{
    self, DESC_OWN, ETH_FCS_LEN, NUM_RX_DESC, R8169_RX_BUF_SIZE,
    RX_CONFIG, RX_CONFIG_ACCEPT_MASK, RX_CRC, RX_CS_FAIL_MASK, RX_PROTO_MASK,
    RX_PROTO_TCP, RX_PROTO_UDP, RX_RES, RX_RUNT, RX_RWT, RX_VLAN_TAG, Mmio,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Mask for extracting packet size from opts1 (bits 13:0).
const PKT_SIZE_MASK: u32 = 0x3FFF;

/// The DMA buffer size for each RX slot. We use `PAGE_SIZE` pages that
/// cover at least `R8169_RX_BUF_SIZE` bytes. The hardware is told the
/// buffer is `R8169_RX_BUF_SIZE` bytes via the descriptor opts1 field.
const RX_DMA_BUF_PAGES: usize = {
    let buf = R8169_RX_BUF_SIZE as usize;
    (buf + ostd::mm::PAGE_SIZE - 1) / ostd::mm::PAGE_SIZE
};

// ---------------------------------------------------------------------------
// RX checksum status
// ---------------------------------------------------------------------------

/// Result of hardware RX checksum validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxCsumStatus {
    /// Hardware verified the checksum and it is correct.
    Unnecessary,
    /// Hardware did not validate the checksum (or the protocol is unknown).
    None,
}

/// Validates the RX checksum based on opts1.
///
/// If the protocol is TCP or UDP and no checksum-fail bits are set,
/// the checksum is marked as unnecessary (hardware-verified).
///
/// Corresponds to `rtl8169_rx_csum` in the C driver.
pub fn rx_csum(opts1: u32) -> RxCsumStatus {
    let status = opts1 & (RX_PROTO_MASK | RX_CS_FAIL_MASK);
    if status == RX_PROTO_TCP || status == RX_PROTO_UDP {
        RxCsumStatus::Unnecessary
    } else {
        RxCsumStatus::None
    }
}

// ---------------------------------------------------------------------------
// VLAN tag extraction
// ---------------------------------------------------------------------------

/// Extracts the VLAN tag from opts2 if the RxVlanTag bit is set.
///
/// Returns `Some(vid)` with the byte-swapped VLAN ID, or `None`.
///
/// Corresponds to `rtl8169_rx_vlan_tag` in the C driver.
pub fn rx_vlan_tag(opts2: u32) -> Option<u16> {
    if opts2 & RX_VLAN_TAG != 0 {
        // The hardware stores the tag with bytes swapped (swab16).
        let raw = (opts2 & 0xFFFF) as u16;
        Some(raw.swap_bytes())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// RX ring management
// ---------------------------------------------------------------------------

/// Per-slot DMA buffer for an RX descriptor. Each slot holds a
/// `DmaStream<FromDevice>` that the NIC writes received data into.
pub struct RxDmaBuf {
    /// The DMA stream backing this receive buffer.
    pub stream: Arc<DmaStream<FromDevice>>,
}

impl RxDmaBuf {
    /// Allocates a new RX DMA buffer large enough for `R8169_RX_BUF_SIZE`.
    pub fn alloc() -> Result<Self, ostd::Error> {
        let segment = FrameAllocOptions::new().alloc_segment(RX_DMA_BUF_PAGES)?;
        let stream = DmaStream::<FromDevice>::map(segment.into(), false)?;
        Ok(Self {
            stream: Arc::new(stream),
        })
    }

    /// Returns the DMA (bus) address of the buffer start.
    pub fn dma_addr(&self) -> Daddr {
        self.stream.daddr()
    }

    /// Syncs the first `len` bytes from device to CPU and copies
    /// them into the provided slice. Returns the number of bytes copied.
    pub fn read_packet(&self, len: usize, dst: &mut [u8]) -> Result<usize, ostd::Error> {
        let copy_len = len.min(dst.len());
        self.stream.sync_from_device(0..copy_len)?;
        let mut reader = self.stream.reader()?;
        let mut writer = ostd::mm::VmWriter::from(&mut dst[..copy_len] as &mut [u8]);
        reader.read(&mut writer);
        Ok(copy_len)
    }
}

// ---------------------------------------------------------------------------
// RX ring state
// ---------------------------------------------------------------------------

/// State for the receive descriptor ring.
///
/// Owns the descriptor ring, per-slot DMA buffers, and the current
/// consumer index (`cur_rx`).
pub struct RxRing {
    /// The descriptor ring (DMA-coherent memory).
    pub ring: DescRing,
    /// Per-slot receive DMA buffers.
    pub slots: [Option<RxDmaBuf>; NUM_RX_DESC],
    /// Current receive index (consumer side).
    pub cur_rx: u32,
}

/// Helper to create the initial empty slots array.
fn empty_rx_slots() -> [Option<RxDmaBuf>; NUM_RX_DESC] {
    // We cannot use `[None; N]` because `RxDmaBuf` is not Copy.
    // Use array::from_fn instead.
    core::array::from_fn(|_| None)
}

impl RxRing {
    /// Creates a new RX ring with `NUM_RX_DESC` descriptors.
    /// The ring is allocated but not yet filled with buffers.
    pub fn new() -> Result<Self, ostd::Error> {
        let ring = DescRing::new(NUM_RX_DESC)?;
        Ok(Self {
            ring,
            slots: empty_rx_slots(),
            cur_rx: 0,
        })
    }

    /// Allocates a DMA buffer for one RX descriptor slot, writes its
    /// address into the descriptor, and marks the descriptor as owned
    /// by hardware.
    ///
    /// Corresponds to `rtl8169_alloc_rx_data` in the C driver.
    fn alloc_rx_data(&mut self, index: usize) -> Result<(), ostd::Error> {
        let buf = RxDmaBuf::alloc()?;
        let dma_addr = buf.dma_addr();

        // Write the descriptor: addr fields and then mark_to_asic
        let desc = RawDesc {
            opts1: 0,
            opts2: 0,
            addr_lo: dma_addr as u32,
            addr_hi: (dma_addr >> 32) as u32,
        };
        self.ring.write_desc(index, &desc)?;
        mark_to_asic(&self.ring, index)?;

        self.slots[index] = Some(buf);
        Ok(())
    }

    /// Fills the entire RX ring with fresh DMA buffers and sets the
    /// RingEnd bit on the last descriptor.
    ///
    /// On failure, clears any partially-allocated buffers.
    ///
    /// Corresponds to `rtl8169_rx_fill` in the C driver.
    pub fn fill(&mut self) -> Result<(), ostd::Error> {
        for i in 0..NUM_RX_DESC {
            if let Err(e) = self.alloc_rx_data(i) {
                self.clear();
                return Err(e);
            }
        }

        // Mark the last descriptor with RingEnd.
        crate::desc::set_ring_end(&self.ring, NUM_RX_DESC)?;
        Ok(())
    }

    /// Frees all RX DMA buffers and zeroes out descriptors.
    ///
    /// Corresponds to `rtl8169_rx_clear` in the C driver.
    pub fn clear(&mut self) {
        for i in 0..NUM_RX_DESC {
            if self.slots[i].is_some() {
                self.slots[i] = None;
                // Zero out the descriptor
                let _ = self.ring.write_desc(
                    i,
                    &RawDesc {
                        opts1: 0,
                        opts2: 0,
                        addr_lo: 0,
                        addr_hi: 0,
                    },
                );
            }
        }
    }

    /// Re-marks all descriptors as owned by hardware (used during
    /// ring reset/reinit without reallocating buffers).
    pub fn remark_all(&self) -> Result<(), ostd::Error> {
        for i in 0..NUM_RX_DESC {
            mark_to_asic(&self.ring, i)?;
        }
        Ok(())
    }

    /// Resets the consumer index to zero.
    pub fn reset_index(&mut self) {
        self.cur_rx = 0;
    }

    /// Returns the DMA (bus) address of the descriptor ring, for
    /// programming into `RxDescAddrLow` / `RxDescAddrHigh`.
    pub fn ring_dma_addr(&self) -> Daddr {
        self.ring.dma_addr()
    }
}

// ---------------------------------------------------------------------------
// Receive polling
// ---------------------------------------------------------------------------

/// Statistics collected during a single `rtl_rx` poll invocation.
#[derive(Debug, Default)]
pub struct RxPollStats {
    /// Number of packets successfully received.
    pub rx_packets: u32,
    /// Total bytes received (payload, excluding FCS).
    pub rx_bytes: u64,
    /// Number of error frames.
    pub rx_errors: u32,
    /// Number of length errors (runt / watchdog timeout).
    pub rx_length_errors: u32,
    /// Number of CRC errors.
    pub rx_crc_errors: u32,
    /// Number of dropped frames (fragmented, alloc failure, etc).
    pub rx_dropped: u32,
    /// Number of multicast packets received.
    pub rx_multicast: u32,
}

/// Outcome of processing a single RX descriptor.
pub enum RxOutcome {
    /// A complete packet was received, with its data copied into `packet`.
    Packet {
        /// The raw packet bytes (after FCS removal).
        packet: alloc::vec::Vec<u8>,
        /// Hardware checksum status.
        csum: RxCsumStatus,
        /// VLAN tag if present.
        vlan: Option<u16>,
    },
    /// The descriptor was still owned by hardware -- stop polling.
    HwOwned,
    /// The descriptor contained an error or fragment -- skip it.
    Skipped,
}

/// Polls the RX ring for up to `budget` completed descriptors,
/// copying received packet data out and recycling descriptors
/// back to hardware.
///
/// Returns the number of packets processed and cumulative stats.
///
/// This is the core receive path, corresponding to `rtl_rx` in
/// the C driver.
pub fn rtl_rx(
    rx_ring: &mut RxRing,
    budget: usize,
) -> (alloc::vec::Vec<alloc::vec::Vec<u8>>, RxPollStats) {
    let mut stats = RxPollStats::default();
    let mut packets = alloc::vec::Vec::new();

    for _ in 0..budget {
        let entry = (rx_ring.cur_rx as usize) % NUM_RX_DESC;

        // Read opts1 from the descriptor.
        let status = match rx_ring.ring.read_opts1(entry) {
            Ok(s) => s,
            Err(_) => break,
        };

        // If the descriptor is still owned by hardware, stop.
        if status & DESC_OWN != 0 {
            break;
        }

        // Read the full descriptor to get opts2 (for VLAN).
        let desc = match rx_ring.ring.read_desc(entry) {
            Ok(d) => d,
            Err(_) => break,
        };

        // Check for RX errors (RxRES bit).
        if status & RX_RES != 0 {
            stats.rx_errors += 1;
            if status & (RX_RWT | RX_RUNT) != 0 {
                stats.rx_length_errors += 1;
            }
            if status & RX_CRC != 0 {
                stats.rx_crc_errors += 1;
            }
            // Release the descriptor back to hardware and continue.
            let _ = mark_to_asic(&rx_ring.ring, entry);
            rx_ring.cur_rx = rx_ring.cur_rx.wrapping_add(1);
            continue;
        }

        // Extract packet size from bits 13:0.
        let mut pkt_size = (status & PKT_SIZE_MASK) as usize;

        // Strip FCS (we do not pass it up).
        if pkt_size > ETH_FCS_LEN {
            pkt_size -= ETH_FCS_LEN;
        } else {
            // Degenerate packet -- drop.
            let _ = mark_to_asic(&rx_ring.ring, entry);
            rx_ring.cur_rx = rx_ring.cur_rx.wrapping_add(1);
            stats.rx_dropped += 1;
            continue;
        }

        // Reject fragmented frames (symptom of over-MTU frames).
        if is_fragmented_frame(status) {
            stats.rx_dropped += 1;
            stats.rx_length_errors += 1;
            let _ = mark_to_asic(&rx_ring.ring, entry);
            rx_ring.cur_rx = rx_ring.cur_rx.wrapping_add(1);
            continue;
        }

        // Read the packet data from the DMA buffer.
        if let Some(ref buf) = rx_ring.slots[entry] {
            let mut pkt_data = alloc::vec![0u8; pkt_size];
            match buf.read_packet(pkt_size, &mut pkt_data) {
                Ok(n) if n == pkt_size => {
                    let csum = rx_csum(status);
                    let _vlan = rx_vlan_tag(desc.opts2);

                    stats.rx_packets += 1;
                    stats.rx_bytes += pkt_size as u64;

                    packets.push(pkt_data);
                }
                _ => {
                    stats.rx_dropped += 1;
                }
            }
        } else {
            stats.rx_dropped += 1;
        }

        // Release descriptor back to hardware.
        let _ = mark_to_asic(&rx_ring.ring, entry);
        rx_ring.cur_rx = rx_ring.cur_rx.wrapping_add(1);
    }

    (packets, stats)
}

/// Convenience wrapper that polls `rtl_rx` and returns the first
/// available packet as an `RxBuffer`-compatible byte vector, or
/// `NetError::NotReady` if no packet is available.
///
/// This is intended to be called from the `AnyNetworkDevice::receive`
/// implementation.
pub fn receive_one(rx_ring: &mut RxRing) -> Result<alloc::vec::Vec<u8>, NetError> {
    let (packets, _stats) = rtl_rx(rx_ring, 1);
    packets.into_iter().next().ok_or(NetError::NotReady)
}

// ---------------------------------------------------------------------------
// RX close (disable acceptance)
// ---------------------------------------------------------------------------

/// Disables RX acceptance by clearing the accept mask bits in RxConfig.
///
/// Corresponds to `rtl_rx_close` in the C driver.
pub fn rx_close(mmio: &Mmio) -> Result<(), ostd::Error> {
    let rx_config = mmio.read32(RX_CONFIG)?;
    mmio.write32(RX_CONFIG, rx_config & !RX_CONFIG_ACCEPT_MASK)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// RX config helpers
// ---------------------------------------------------------------------------

/// Sets the RxMaxSize register to `R8169_RX_BUF_SIZE + 1`.
///
/// Corresponds to `rtl_set_rx_max_size` in the C driver.
pub fn set_rx_max_size(mmio: &Mmio) -> Result<(), ostd::Error> {
    mmio.write16(regs::RX_MAX_SIZE, (R8169_RX_BUF_SIZE + 1) as u16)
}

/// Programs the RX descriptor ring address into the hardware registers.
pub fn set_rx_desc_addr(mmio: &Mmio, dma_addr: Daddr) -> Result<(), ostd::Error> {
    mmio.write32(regs::RX_DESC_ADDR_HIGH, (dma_addr >> 32) as u32)?;
    mmio.write32(regs::RX_DESC_ADDR_LOW, (dma_addr & 0xFFFF_FFFF) as u32)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rx_csum_tcp() {
        // TCP protocol, no fail bits
        assert_eq!(rx_csum(RX_PROTO_TCP), RxCsumStatus::Unnecessary);
    }

    #[test]
    fn test_rx_csum_udp() {
        // UDP protocol, no fail bits
        assert_eq!(rx_csum(RX_PROTO_UDP), RxCsumStatus::Unnecessary);
    }

    #[test]
    fn test_rx_csum_with_fail() {
        use crate::regs::{IP_FAIL, TCP_FAIL};
        // TCP with IP fail -- should not pass as Unnecessary
        assert_eq!(rx_csum(RX_PROTO_TCP | IP_FAIL), RxCsumStatus::None);
    }

    #[test]
    fn test_rx_csum_unknown_proto() {
        assert_eq!(rx_csum(0), RxCsumStatus::None);
    }

    #[test]
    fn test_rx_vlan_tag_present() {
        let opts2 = RX_VLAN_TAG | 0x0064; // VLAN ID 100 byte-swapped
        let vlan = rx_vlan_tag(opts2);
        assert!(vlan.is_some());
        assert_eq!(vlan.unwrap(), 0x0064u16.swap_bytes());
    }

    #[test]
    fn test_rx_vlan_tag_absent() {
        assert_eq!(rx_vlan_tag(0), None);
    }

    #[test]
    fn test_fragmented_frame_detection() {
        use crate::regs::{FIRST_FRAG, LAST_FRAG};
        // Complete frame: both FirstFrag and LastFrag set
        assert!(!is_fragmented_frame(FIRST_FRAG | LAST_FRAG));
        // Only first frag
        assert!(is_fragmented_frame(FIRST_FRAG));
        // Only last frag
        assert!(is_fragmented_frame(LAST_FRAG));
        // Neither
        assert!(is_fragmented_frame(0));
    }
}
