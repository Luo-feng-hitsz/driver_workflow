// SPDX-License-Identifier: MPL-2.0

//! Transmit path for the RTL8168g (r8169) network controller.
//!
//! This module implements the TX descriptor ring management, packet
//! transmission (`start_xmit`), DMA buffer mapping (`tx_map`),
//! TX completion processing (`rtl_tx`), checksum/TSO offload helpers
//! (v2 only -- RTL8168g uses `tso_csum_v2`), VLAN tag insertion,
//! TX ring cleanup, the doorbell write (`TxPoll`), and available-slot
//! checking.
//!
//! The design follows the same patterns established by `rx.rs`:
//!   - `TxRing` owns the descriptor ring and per-slot DMA buffer metadata.
//!   - Public helper functions operate on `TxRing` and `Mmio`.
//!   - No `unsafe` beyond the transmute in `DescRing` (which lives in `desc.rs`).
//!
//! Translated from: drivers/net/ethernet/realtek/r8169_main.c

use alloc::sync::Arc;

use aster_network::NetError;
use ostd::mm::{
    Daddr, FrameAllocOptions, HasDaddr,
    dma::{DmaStream, ToDevice},
    io::util::HasVmReaderWriter,
};

use crate::desc::{DescRing, RawDesc, set_ring_end, tx_slots_avail, TxSlot};
use crate::regs::{
    self, DESC_OWN, FIRST_FRAG, LAST_FRAG, NPQ, NUM_TX_DESC, RING_END,
    R8169_TX_START_THRS, R8169_TX_STOP_THRS, TX_VLAN_TAG, Mmio,
    // TSO / checksum v2 bits (used by RTL8168g)
    TD1_GTSEN_V4, TD1_GTSEN_V6, TD1_IPV4_CS, TD1_IPV6_CS,
    TD1_MSS_SHIFT, TD1_TCP_CS, TD1_UDP_CS, GTTCPHO_SHIFT,
    TCPHO_SHIFT,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum Ethernet frame size (60 bytes, excluding FCS).
/// The hardware may misbehave if frames shorter than this are submitted
/// without padding.  RTL8168g (MAC_VER_40) does NOT need the extra
/// `rtl_quirk_packet_padto` logic (that is for MAC_VER_34 and 8125+),
/// but we still pad to `ETH_ZLEN` for correctness.
const ETH_ZLEN: usize = 60;

/// Number of pages needed for a maximum-length TX DMA buffer.
/// We allocate a single page which is 4096 bytes -- more than enough
/// for any non-TSO Ethernet frame (max 1514 bytes standard, 9014 jumbo).
const TX_DMA_BUF_PAGES: usize = 1;

// ---------------------------------------------------------------------------
// TX DMA buffer
// ---------------------------------------------------------------------------

/// A DMA-mapped buffer used to hold outgoing packet data.
///
/// The driver copies the packet payload into this buffer, then programs
/// its DMA address into the TX descriptor.  The buffer is freed (dropped)
/// when TX completion reclaims the descriptor slot.
pub struct TxDmaBuf {
    /// The DMA stream backing this transmit buffer.
    pub stream: Arc<DmaStream<ToDevice>>,
    /// The actual number of payload bytes written.
    pub len: usize,
}

impl TxDmaBuf {
    /// Allocates a new TX DMA buffer and copies `data` into it.
    ///
    /// The buffer is synced to device after the copy so the NIC can
    /// read it immediately.
    pub fn new(data: &[u8]) -> Result<Self, ostd::Error> {
        let segment = FrameAllocOptions::new().alloc_segment(TX_DMA_BUF_PAGES)?;
        let stream = DmaStream::<ToDevice>::map(segment.into(), false)?;
        let len = data.len();

        // Copy packet data into the DMA buffer.
        {
            let mut writer = stream.writer()?;
            let mut reader = ostd::mm::VmReader::from(data);
            writer.write(&mut reader);
        }
        stream.sync_to_device(0..len)?;

        Ok(Self {
            stream: Arc::new(stream),
            len,
        })
    }

    /// Returns the DMA (bus) address of the buffer.
    pub fn dma_addr(&self) -> Daddr {
        self.stream.daddr()
    }
}

// ---------------------------------------------------------------------------
// TX ring
// ---------------------------------------------------------------------------

/// State for the transmit descriptor ring.
///
/// Owns the descriptor ring, per-slot metadata (DMA buffer references),
/// and the producer/consumer indexes (`cur_tx`, `dirty_tx`).
pub struct TxRing {
    /// The descriptor ring (DMA-coherent memory).
    pub ring: DescRing,
    /// Per-slot transmit buffer metadata.
    pub slots: [TxSlot; NUM_TX_DESC],
    /// Next descriptor to be filled by the driver (producer).
    pub cur_tx: u32,
    /// Next descriptor to be reclaimed after TX completion (consumer).
    pub dirty_tx: u32,
}

/// Helper to create the initial empty TX slots array.
fn empty_tx_slots() -> [TxSlot; NUM_TX_DESC] {
    core::array::from_fn(|_| TxSlot::new())
}

impl TxRing {
    /// Creates a new TX ring with `NUM_TX_DESC` descriptors.
    ///
    /// The ring is allocated and zeroed but no DMA buffers are attached.
    pub fn new() -> Result<Self, ostd::Error> {
        let ring = DescRing::new(NUM_TX_DESC)?;
        // Set the RingEnd bit on the last descriptor.
        set_ring_end(&ring, NUM_TX_DESC)?;

        Ok(Self {
            ring,
            slots: empty_tx_slots(),
            cur_tx: 0,
            dirty_tx: 0,
        })
    }

    /// Returns the number of available (free) TX descriptor slots.
    pub fn slots_avail(&self) -> usize {
        tx_slots_avail(self.dirty_tx, self.cur_tx)
    }

    /// Returns `true` if there are enough free slots to accept a new packet.
    pub fn can_send(&self) -> bool {
        self.slots_avail() >= R8169_TX_STOP_THRS
    }

    /// Resets both producer and consumer indexes to zero.
    pub fn reset_indexes(&mut self) {
        self.cur_tx = 0;
        self.dirty_tx = 0;
    }

    /// Returns the DMA (bus) address of the descriptor ring, for
    /// programming into `TxDescAddrLow` / `TxDescAddrHigh`.
    pub fn ring_dma_addr(&self) -> Daddr {
        self.ring.dma_addr()
    }

    /// Clears all TX slots, releasing any held DMA buffer references
    /// and zeroing descriptors.
    ///
    /// Corresponds to `rtl8169_tx_clear` in the C driver.
    pub fn clear(&mut self) {
        for i in 0..NUM_TX_DESC {
            self.slots[i].clear();
            let _ = self.ring.write_desc(
                i,
                &RawDesc {
                    opts1: if i == NUM_TX_DESC - 1 { RING_END } else { 0 },
                    opts2: 0,
                    addr_lo: 0,
                    addr_hi: 0,
                },
            );
        }
        self.cur_tx = 0;
        self.dirty_tx = 0;
    }

    /// Unmaps a single TX descriptor slot, clearing the descriptor and
    /// releasing the DMA buffer reference.
    ///
    /// Corresponds to `rtl8169_unmap_tx_skb` in the C driver.
    fn unmap_tx_slot(&mut self, entry: usize) {
        self.slots[entry].clear();
        // Preserve the RingEnd bit when zeroing.
        let eor = if entry == NUM_TX_DESC - 1 {
            RING_END
        } else {
            0
        };
        let _ = self.ring.write_desc(
            entry,
            &RawDesc {
                opts1: eor,
                opts2: 0,
                addr_lo: 0,
                addr_hi: 0,
            },
        );
    }
}

// ---------------------------------------------------------------------------
// VLAN tag helper
// ---------------------------------------------------------------------------

/// Computes the opts2 VLAN tag word for a TX descriptor.
///
/// In a full driver this would extract the VLAN TCI from the packet
/// metadata.  For the initial Asterinas bring-up we do not support
/// hardware VLAN insertion, so this always returns 0.
///
/// Corresponds to `rtl8169_tx_vlan_tag` in the C driver.
pub fn tx_vlan_tag(_packet: &[u8]) -> u32 {
    // VLAN offload not yet wired up -- no tag insertion.
    0
}

// ---------------------------------------------------------------------------
// Checksum / TSO offload (v2, for RTL8168g)
// ---------------------------------------------------------------------------

/// Offload descriptor flags computed from packet metadata.
///
/// In the full Linux driver, `rtl8169_tso_csum_v2` inspects the skb
/// for GSO (TSO) segments and checksum-partial requests and sets bits
/// in `opts[0]` and `opts[1]`.
///
/// For the initial Asterinas bring-up we do not perform hardware
/// checksum offload or TSO -- the network stack computes checksums in
/// software.  This function returns `[0, 0]` which means "no offload,
/// no TSO".
///
/// When HW offload is wired up later, this function should inspect the
/// packet headers and set the appropriate `TD1_*` bits.
///
/// Corresponds to `rtl8169_tso_csum_v2` in the C driver.
pub fn tso_csum_v2(_packet: &[u8]) -> [u32; 2] {
    [0, 0]
}

// ---------------------------------------------------------------------------
// TX map: write one descriptor
// ---------------------------------------------------------------------------

/// Maps a packet's DMA buffer into a single TX descriptor.
///
/// Sets opts1 (length, RingEnd if last descriptor, optionally DescOwn)
/// and opts2 in the descriptor at `entry`.
///
/// Corresponds to `rtl8169_tx_map` in the C driver (single-fragment case).
fn tx_map(
    tx_ring: &mut TxRing,
    opts: &[u32; 2],
    dma_buf: &TxDmaBuf,
    entry: usize,
    desc_own: bool,
) -> Result<(), ostd::Error> {
    let dma_addr = dma_buf.dma_addr();
    let len = dma_buf.len as u32;

    let mut opts1 = opts[0] | len;
    if entry == NUM_TX_DESC - 1 {
        opts1 |= RING_END;
    }
    if desc_own {
        opts1 |= DESC_OWN;
    }

    let desc = RawDesc {
        opts1,
        opts2: opts[1],
        addr_lo: dma_addr as u32,
        addr_hi: (dma_addr >> 32) as u32,
    };
    tx_ring.ring.write_desc(entry, &desc)?;

    tx_ring.slots[entry].len = len;
    tx_ring.slots[entry].dma_buf = Some(dma_buf.stream.clone());

    Ok(())
}

// ---------------------------------------------------------------------------
// Doorbell
// ---------------------------------------------------------------------------

/// Rings the TX doorbell to tell the NIC to check for new descriptors.
///
/// For RTL8168g (non-8125) this writes `NPQ` to the `TxPoll` register.
///
/// Corresponds to `rtl8169_doorbell` in the C driver.
pub fn doorbell(mmio: &Mmio) -> Result<(), ostd::Error> {
    mmio.write8(regs::TX_POLL, NPQ)
}

// ---------------------------------------------------------------------------
// TX address programming
// ---------------------------------------------------------------------------

/// Programs the TX descriptor ring address into the hardware registers.
pub fn set_tx_desc_addr(mmio: &Mmio, dma_addr: Daddr) -> Result<(), ostd::Error> {
    mmio.write32(regs::TX_DESC_START_ADDR_HIGH, (dma_addr >> 32) as u32)?;
    mmio.write32(regs::TX_DESC_START_ADDR_LOW, (dma_addr & 0xFFFF_FFFF) as u32)?;
    Ok(())
}

/// Sets the MaxTxPacketSize register.
///
/// For RTL8168g, `TX_PACKET_MAX` is `(8064 >> 7)` = 63.
pub fn set_tx_max_size(mmio: &Mmio) -> Result<(), ostd::Error> {
    mmio.write8(regs::MAX_TX_PACKET_SIZE, regs::TX_PACKET_MAX)
}

// ---------------------------------------------------------------------------
// Packet padding
// ---------------------------------------------------------------------------

/// Pads a packet to the minimum Ethernet frame length (`ETH_ZLEN` = 60 bytes)
/// if it is shorter.
///
/// Returns a `Vec<u8>` that is either the original data (if already long
/// enough) or a zero-padded copy.
///
/// For RTL8168g (MAC_VER_40) the only relevant quirk is the general
/// `ETH_ZLEN` padding; the UDP PTP padding (`rtl8125_quirk_udp_padto`)
/// applies only to RTL8125 variants.
fn pad_packet(data: &[u8]) -> alloc::vec::Vec<u8> {
    if data.len() >= ETH_ZLEN {
        data.to_vec()
    } else {
        let mut padded = alloc::vec![0u8; ETH_ZLEN];
        padded[..data.len()].copy_from_slice(data);
        padded
    }
}

// ---------------------------------------------------------------------------
// Start xmit (single-buffer, no frags)
// ---------------------------------------------------------------------------

/// Transmits a single packet through the TX ring.
///
/// This is the primary entry point called from the `AnyNetworkDevice::send`
/// implementation.  It handles:
///   1. Checking for available descriptor slots.
///   2. Computing offload flags (currently none).
///   3. Padding the packet to `ETH_ZLEN` if needed.
///   4. Allocating a DMA buffer and copying packet data.
///   5. Writing the TX descriptor with `FirstFrag | LastFrag`.
///   6. Advancing `cur_tx`.
///   7. Ringing the doorbell.
///
/// In the Linux driver this corresponds to `rtl8169_start_xmit`.
/// We simplify by not supporting scatter-gather (no `xmit_frags`) --
/// the Asterinas network stack hands us a contiguous `&[u8]`.
pub fn start_xmit(
    tx_ring: &mut TxRing,
    mmio: &Mmio,
    packet: &[u8],
) -> Result<(), NetError> {
    // 1. Check for available slots.
    if !tx_ring.can_send() {
        return Err(NetError::Busy);
    }

    let entry = (tx_ring.cur_tx as usize) % NUM_TX_DESC;

    // 2. Compute offload opts (VLAN tag in opts[1], TSO/csum in opts[0]).
    let vlan = tx_vlan_tag(packet);
    let mut opts = tso_csum_v2(packet);
    opts[1] |= vlan;

    // 3. Pad the packet if necessary.
    let padded = pad_packet(packet);

    // 4. Allocate a DMA buffer and copy data.
    let dma_buf = TxDmaBuf::new(&padded).map_err(|_| NetError::NoMemory)?;

    // 5. Set FirstFrag in opts (we add LastFrag after mapping).
    //    For a single-buffer packet both FirstFrag and LastFrag are set,
    //    and we set DescOwn so the NIC takes ownership immediately.
    opts[0] |= FIRST_FRAG | LAST_FRAG;

    // Map the descriptor with DescOwn set.
    tx_map(tx_ring, &opts, &dma_buf, entry, true).map_err(|_| NetError::NoMemory)?;

    // Record that this slot holds the last (and only) fragment.
    tx_ring.slots[entry].is_last = true;
    // Keep the DMA buffer alive until TX completion.
    tx_ring.slots[entry].dma_buf = Some(dma_buf.stream.clone());

    // 6. Advance cur_tx (one descriptor consumed, no frags).
    tx_ring.cur_tx = tx_ring.cur_tx.wrapping_add(1);

    // 7. Ring the doorbell to notify the NIC.
    let _ = doorbell(mmio);

    Ok(())
}

// ---------------------------------------------------------------------------
// TX completion
// ---------------------------------------------------------------------------

/// Statistics collected during a single `rtl_tx` completion pass.
#[derive(Debug, Default)]
pub struct TxCompleteStats {
    /// Number of packets whose transmission completed.
    pub tx_packets: u32,
    /// Total bytes transmitted.
    pub tx_bytes: u64,
}

/// Processes completed TX descriptors, releasing DMA buffers and
/// advancing `dirty_tx`.
///
/// Should be called periodically (e.g., from the poll/NAPI handler
/// or from `free_processed_tx_buffers`).
///
/// Returns statistics for the completed descriptors and whether
/// the ring has newly available slots (for waking the queue).
///
/// Corresponds to `rtl_tx` in the C driver.
pub fn rtl_tx(tx_ring: &mut TxRing) -> TxCompleteStats {
    let mut stats = TxCompleteStats::default();
    let mut dirty_tx = tx_ring.dirty_tx;

    while tx_ring.cur_tx != dirty_tx {
        let entry = (dirty_tx as usize) % NUM_TX_DESC;

        // Read opts1 to check the DescOwn bit.
        let status = match tx_ring.ring.read_opts1(entry) {
            Ok(s) => s,
            Err(_) => break,
        };

        // If the NIC still owns the descriptor, stop.
        if status & DESC_OWN != 0 {
            break;
        }

        // Collect stats if this slot was the last fragment of a packet.
        if tx_ring.slots[entry].is_last {
            stats.tx_packets += 1;
            stats.tx_bytes += tx_ring.slots[entry].len as u64;
        }

        // Release the DMA buffer and clear the slot.
        tx_ring.unmap_tx_slot(entry);

        dirty_tx = dirty_tx.wrapping_add(1);
    }

    tx_ring.dirty_tx = dirty_tx;

    // If the ring was previously full and now has space, ring the
    // doorbell again (the "8168 hack" from the C driver).
    // The caller (driver layer) can use `can_send()` to decide
    // whether to wake the transmit queue.

    stats
}

// ---------------------------------------------------------------------------
// TX close / reset helpers
// ---------------------------------------------------------------------------

/// Clears the entire TX ring, releasing all pending DMA buffers.
///
/// Intended for use during device close or reset.
///
/// Corresponds to `rtl8169_tx_clear` in the C driver.
pub fn tx_clear(tx_ring: &mut TxRing) {
    tx_ring.clear();
}

/// Re-rings the doorbell if there are still pending descriptors
/// that the NIC has not yet processed (the "8168 hack").
///
/// Call after `rtl_tx` returns with completed packets.
pub fn tx_kick_if_pending(tx_ring: &TxRing, mmio: &Mmio) {
    if tx_ring.cur_tx != tx_ring.dirty_tx {
        let _ = doorbell(mmio);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pad_packet_short() {
        let short = [0xAAu8; 30];
        let padded = pad_packet(&short);
        assert_eq!(padded.len(), ETH_ZLEN);
        // First 30 bytes should match.
        assert_eq!(&padded[..30], &short);
        // Remaining bytes should be zero.
        assert!(padded[30..].iter().all(|&b| b == 0));
    }

    #[test]
    fn test_pad_packet_exact() {
        let exact = [0xBBu8; ETH_ZLEN];
        let padded = pad_packet(&exact);
        assert_eq!(padded.len(), ETH_ZLEN);
        assert_eq!(&padded[..], &exact[..]);
    }

    #[test]
    fn test_pad_packet_long() {
        let long = [0xCCu8; 1500];
        let padded = pad_packet(&long);
        assert_eq!(padded.len(), 1500);
    }

    #[test]
    fn test_vlan_tag_none() {
        assert_eq!(tx_vlan_tag(&[0u8; 64]), 0);
    }

    #[test]
    fn test_tso_csum_v2_no_offload() {
        let opts = tso_csum_v2(&[0u8; 64]);
        assert_eq!(opts, [0, 0]);
    }
}
