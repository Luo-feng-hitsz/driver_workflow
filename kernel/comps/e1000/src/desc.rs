// SPDX-License-Identifier: MPL-2.0

//! TX/RX descriptor structures and descriptor ring management for the e1000.
//! Translated from e1000_hw.h (struct e1000_tx_desc, e1000_rx_desc, e1000_context_desc)
//! and e1000.h (ring types).

use ostd_pod::Pod;

// ============================================================================
// Legacy Receive Descriptor
// ============================================================================

/// Legacy receive descriptor (16 bytes), matching the hardware layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct E1000RxDesc {
    /// Physical address of the receive buffer.
    pub buffer_addr: u64,
    /// Length of received data in bytes.
    pub length: u16,
    /// Packet checksum.
    pub csum: u16,
    /// Descriptor status bits (DD, EOP, etc.).
    pub status: u8,
    /// Descriptor error bits.
    pub errors: u8,
    /// Special field (VLAN tag).
    pub special: u16,
}

impl E1000RxDesc {
    pub const SIZE: usize = core::mem::size_of::<Self>();

    /// Returns true if the hardware has finished writing to this descriptor.
    #[inline]
    pub fn done(&self) -> bool {
        self.status & crate::regs::RXD_STAT_DD != 0
    }

    /// Returns true if this is the last descriptor in a packet.
    #[inline]
    pub fn end_of_packet(&self) -> bool {
        self.status & crate::regs::RXD_STAT_EOP != 0
    }

    /// Returns true if any frame-level error bit is set.
    #[inline]
    pub fn has_error(&self) -> bool {
        self.errors & crate::regs::RXD_ERR_FRAME_ERR_MASK != 0
    }
}

// ============================================================================
// Legacy Transmit Descriptor
// ============================================================================

/// Legacy transmit descriptor (16 bytes), matching the hardware layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct E1000TxDesc {
    /// Physical address of the transmit buffer.
    pub buffer_addr: u64,
    /// Lower data: length (16 bits) | cso (8 bits) | cmd (8 bits) packed as u32.
    pub lower: u32,
    /// Upper data: status (8 bits) | css (8 bits) | special (16 bits) packed as u32.
    pub upper: u32,
}

impl E1000TxDesc {
    pub const SIZE: usize = core::mem::size_of::<Self>();

    /// Returns true if the hardware has processed this descriptor (DD bit set).
    #[inline]
    pub fn done(&self) -> bool {
        // DD is bit 0 of the status byte, which is the lowest byte of `upper`.
        (self.upper & 0x01) != 0
    }

    /// Creates a legacy transmit descriptor for a data buffer.
    pub fn new_data(buffer_addr: u64, length: u16, cmd: u8) -> Self {
        Self {
            buffer_addr,
            lower: (length as u32) | ((cmd as u32) << 24),
            upper: 0,
        }
    }
}

// ============================================================================
// Context Descriptor (for TSO/checksum offload)
// ============================================================================

/// Offload context descriptor (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct E1000ContextDesc {
    pub lower_setup: u32,
    pub upper_setup: u32,
    pub cmd_and_length: u32,
    pub tcp_seg_setup: u32,
}

// ============================================================================
// Descriptor Ring
// ============================================================================

/// Manages a ring of descriptors with head/tail tracking.
pub struct DescRing<T: Pod + Copy> {
    /// Number of descriptors in the ring (power of 2 recommended).
    count: usize,
    /// Next descriptor to use (software write pointer).
    next_to_use: usize,
    /// Next descriptor to check for completion (software read pointer).
    next_to_clean: usize,
    /// Phantom type marker.
    _marker: core::marker::PhantomData<T>,
}

impl<T: Pod + Copy> DescRing<T> {
    /// Creates a new descriptor ring tracker with `count` entries.
    pub fn new(count: usize) -> Self {
        Self {
            count,
            next_to_use: 0,
            next_to_clean: 0,
            _marker: core::marker::PhantomData,
        }
    }

    /// Returns the ring size (number of descriptors).
    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    /// Returns the next-to-use index.
    #[inline]
    pub fn next_to_use(&self) -> usize {
        self.next_to_use
    }

    /// Returns the next-to-clean index.
    #[inline]
    pub fn next_to_clean(&self) -> usize {
        self.next_to_clean
    }

    /// Advances next_to_use by one, wrapping around.
    #[inline]
    pub fn advance_use(&mut self) {
        self.next_to_use = (self.next_to_use + 1) % self.count;
    }

    /// Advances next_to_clean by one, wrapping around.
    #[inline]
    pub fn advance_clean(&mut self) {
        self.next_to_clean = (self.next_to_clean + 1) % self.count;
    }

    /// Sets the next_to_use index explicitly.
    #[inline]
    pub fn set_next_to_use(&mut self, val: usize) {
        self.next_to_use = val;
    }

    /// Sets the next_to_clean index explicitly.
    #[inline]
    pub fn set_next_to_clean(&mut self, val: usize) {
        self.next_to_clean = val;
    }

    /// Returns the number of unused descriptors available.
    /// This is equivalent to the Linux E1000_DESC_UNUSED macro.
    #[inline]
    pub fn unused_count(&self) -> usize {
        let clean = self.next_to_clean;
        let used = self.next_to_use;
        if clean > used {
            clean - used - 1
        } else {
            self.count + clean - used - 1
        }
    }

    /// Returns true if the ring is full (only 1 slot left as guard).
    #[inline]
    pub fn is_full(&self) -> bool {
        self.unused_count() == 0
    }
}
