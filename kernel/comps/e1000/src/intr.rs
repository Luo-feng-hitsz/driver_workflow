// SPDX-License-Identifier: MPL-2.0

//! Interrupt handling for the e1000 82540EM.
//! Provides IRQ enable/disable, interrupt cause processing, and
//! NAPI-style polling coordination.
//! Translated from e1000_main.c interrupt-related functions.

use crate::hw::E1000Hw;
use crate::regs::*;

// ============================================================================
// Interrupt Enable / Disable
// ============================================================================

/// Enables interrupts by writing the standard IMS mask.
pub fn irq_enable(hw: &E1000Hw) {
    hw.regs.write(IMS, IMS_ENABLE_MASK);
    // Flush
    let _ = hw.regs.read(STATUS);
}

/// Disables all interrupts by writing all-ones to IMC.
pub fn irq_disable(hw: &E1000Hw) {
    hw.regs.write(IMC, 0xFFFF_FFFF);
    // Flush
    let _ = hw.regs.read(STATUS);
}

// ============================================================================
// Interrupt Cause Processing
// ============================================================================

/// Reads and returns the interrupt cause register (ICR).
/// Reading ICR also clears the interrupt bits (read-to-clear).
#[inline]
pub fn read_icr(hw: &E1000Hw) -> u32 {
    hw.regs.read(ICR)
}

/// Processes an interrupt. Returns flags indicating what happened.
pub fn process_interrupt(hw: &E1000Hw) -> InterruptStatus {
    let icr = read_icr(hw);

    // If no interrupt bits are asserted, this was not our interrupt
    if icr == 0 {
        return InterruptStatus::empty();
    }

    let mut status = InterruptStatus::empty();

    if icr & ICR_LSC != 0 {
        status |= InterruptStatus::LINK_CHANGE;
    }

    if icr & ICR_RXT0 != 0 {
        status |= InterruptStatus::RX;
    }

    if icr & ICR_RXDMT0 != 0 {
        status |= InterruptStatus::RX_DESC_MIN_THRESH;
    }

    if icr & ICR_TXDW != 0 {
        status |= InterruptStatus::TX_DONE;
    }

    if icr & ICR_RXSEQ != 0 {
        status |= InterruptStatus::RX_SEQ_ERR;
    }

    if icr & ICR_RXO != 0 {
        status |= InterruptStatus::RX_OVERRUN;
    }

    status
}

// ============================================================================
// Interrupt Status Flags
// ============================================================================

bitflags::bitflags! {
    /// Flags indicating which interrupt causes were triggered.
    pub struct InterruptStatus: u32 {
        const LINK_CHANGE = 0x01;
        const RX = 0x02;
        const RX_DESC_MIN_THRESH = 0x04;
        const TX_DONE = 0x08;
        const RX_SEQ_ERR = 0x10;
        const RX_OVERRUN = 0x20;
    }
}

// ============================================================================
// Interrupt Throttling
// ============================================================================

/// Sets the interrupt throttle rate (ITR).
/// `itr_val` is in units of 256ns intervals.
/// A value of 0 disables throttling.
pub fn set_itr(hw: &E1000Hw, itr_val: u32) {
    hw.regs.write(ITR, itr_val);
}

/// Computes ITR value from desired interrupts per second.
/// The ITR register uses 256ns increments: ITR = 1_000_000_000 / (256 * ints_per_sec)
pub fn compute_itr(ints_per_sec: u32) -> u32 {
    if ints_per_sec == 0 {
        return 0;
    }
    1_000_000_000 / (256 * ints_per_sec)
}
