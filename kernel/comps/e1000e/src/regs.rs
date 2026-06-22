// SPDX-License-Identifier: MPL-2.0

//! Register offset constants, bit-field definitions, and MMIO accessor
//! for the Intel 82574L (e1000e).
//!
//! The 82574L shares the same register layout as the classic e1000 family
//! (82540EM etc.) for the registers used by this driver.

#![allow(dead_code)]

use ostd::io::IoMem;
use ostd::mm::VmIoOnce;

// ============================================================================
// Register offsets
// ============================================================================

// Device Control
pub const CTRL: usize = 0x00000;
pub const STATUS: usize = 0x00008;
pub const EECD: usize = 0x00010;
pub const EERD: usize = 0x00014;
pub const CTRL_EXT: usize = 0x00018;
pub const MDIC: usize = 0x00020;

// Flow Control
pub const FCAL: usize = 0x00028;
pub const FCAH: usize = 0x0002C;
pub const FCT: usize = 0x00030;
pub const VET: usize = 0x00038;

// Interrupt
pub const ICR: usize = 0x000C0;
pub const ITR: usize = 0x000C4;
pub const ICS: usize = 0x000C8;
pub const IMS: usize = 0x000D0;
pub const IMC: usize = 0x000D8;
pub const IAM: usize = 0x000E0;

// Receive
pub const RCTL: usize = 0x00100;

// Transmit
pub const TCTL: usize = 0x00400;
pub const TCTL_EXT: usize = 0x00404;
pub const TIPG: usize = 0x00410;

// Packet Buffer Allocation
pub const PBA: usize = 0x01000;

// RX Descriptor Ring 0
pub const RDBAL: usize = 0x02800;
pub const RDBAH: usize = 0x02804;
pub const RDLEN: usize = 0x02808;
pub const RDH: usize = 0x02810;
pub const RDT: usize = 0x02818;
pub const RDTR: usize = 0x02820;
pub const RXDCTL: usize = 0x02828;
pub const RADV: usize = 0x0282C;

// TX Descriptor Ring 0
pub const TDBAL: usize = 0x03800;
pub const TDBAH: usize = 0x03804;
pub const TDLEN: usize = 0x03808;
pub const TDH: usize = 0x03810;
pub const TDT: usize = 0x03818;
pub const TIDV: usize = 0x03820;
pub const TXDCTL: usize = 0x03828;
pub const TADV: usize = 0x0382C;

// RX Checksum Control
pub const RXCSUM: usize = 0x05000;

// RX Filter Control
pub const RFCTL: usize = 0x05008;

// Multicast Table Array
pub const MTA: usize = 0x05200;

// Receive Address registers
pub const RA: usize = 0x05400;

// ============================================================================
// CTRL register bits
// ============================================================================
pub const CTRL_FD: u32 = 0x00000001;
pub const CTRL_LRST: u32 = 0x00000008;
pub const CTRL_ASDE: u32 = 0x00000020;
pub const CTRL_SLU: u32 = 0x00000040;
pub const CTRL_RST: u32 = 0x04000000;
pub const CTRL_RFCE: u32 = 0x08000000;
pub const CTRL_TFCE: u32 = 0x10000000;
pub const CTRL_VME: u32 = 0x40000000;
pub const CTRL_PHY_RST: u32 = 0x80000000;

// ============================================================================
// STATUS register bits
// ============================================================================
pub const STATUS_FD: u32 = 0x00000001;
pub const STATUS_LU: u32 = 0x00000002;
pub const STATUS_SPEED_MASK: u32 = 0x000000C0;
pub const STATUS_SPEED_10: u32 = 0x00000000;
pub const STATUS_SPEED_100: u32 = 0x00000040;
pub const STATUS_SPEED_1000: u32 = 0x00000080;

// ============================================================================
// EERD register bits
// ============================================================================
pub const EERD_START: u32 = 0x00000001;
pub const EERD_DONE: u32 = 0x00000010;
pub const EERD_ADDR_SHIFT: u32 = 8;
pub const EERD_DATA_SHIFT: u32 = 16;

// ============================================================================
// Interrupt Cause / Mask bits
// ============================================================================
pub const ICR_TXDW: u32 = 0x00000001;
pub const ICR_TXQE: u32 = 0x00000002;
pub const ICR_LSC: u32 = 0x00000004;
pub const ICR_RXSEQ: u32 = 0x00000008;
pub const ICR_RXDMT0: u32 = 0x00000010;
pub const ICR_RXO: u32 = 0x00000040;
pub const ICR_RXT0: u32 = 0x00000080;
pub const ICR_INT_ASSERTED: u32 = 0x80000000;

/// Standard IMS enable mask (RXT0 | TXDW | RXDMT0 | RXSEQ | LSC)
pub const IMS_ENABLE_MASK: u32 =
    ICR_RXT0 | ICR_TXDW | ICR_RXDMT0 | ICR_RXSEQ | ICR_LSC;

// ============================================================================
// Receive Control (RCTL) bits
// ============================================================================
pub const RCTL_EN: u32 = 0x00000002;
pub const RCTL_SBP: u32 = 0x00000004;
pub const RCTL_UPE: u32 = 0x00000008;
pub const RCTL_MPE: u32 = 0x00000010;
pub const RCTL_LPE: u32 = 0x00000020;
pub const RCTL_BAM: u32 = 0x00008000;
pub const RCTL_SZ_2048: u32 = 0x00000000;
pub const RCTL_SECRC: u32 = 0x04000000;
pub const RCTL_BSEX: u32 = 0x02000000;

// ============================================================================
// Transmit Control (TCTL) bits
// ============================================================================
pub const TCTL_EN: u32 = 0x00000002;
pub const TCTL_PSP: u32 = 0x00000008;
pub const TCTL_CT_SHIFT: u32 = 4;
pub const TCTL_COLD_SHIFT: u32 = 12;

pub const COLLISION_THRESHOLD: u32 = 15;
pub const COLLISION_DISTANCE_FD: u32 = 64;

// ============================================================================
// Transmit Inter-Packet Gap (TIPG)
// ============================================================================
pub const TIPG_IPGT_COPPER: u32 = 8;
pub const TIPG_IPGR1_SHIFT: u32 = 10;
pub const TIPG_IPGR1: u32 = 8;
pub const TIPG_IPGR2_SHIFT: u32 = 20;
pub const TIPG_IPGR2: u32 = 6;

// ============================================================================
// TXDCTL register bits
// ============================================================================
pub const TXDCTL_FULL_TX_DESC_WB: u32 = 0x01010000;

// ============================================================================
// Receive Address High
// ============================================================================
pub const RAH_AV: u32 = 0x80000000;

// ============================================================================
// Number of Multicast Table entries
// ============================================================================
pub const NUM_MTA_REGISTERS: usize = 128;

// ============================================================================
// RFCTL bits
// ============================================================================
pub const RFCTL_EXTEN: u32 = 0x0000_8000;

// ============================================================================
// RXCSUM bits
// ============================================================================
pub const RXCSUM_TUOFL: u32 = 0x0000_0200;
pub const RXCSUM_IPOFL: u32 = 0x0000_0100;

// ============================================================================
// CTRL_EXT bits
// ============================================================================
pub const CTRL_EXT_IAME: u32 = 0x0800_0000;

// ============================================================================
// MMIO accessor wrapper
// ============================================================================

/// A thin wrapper over IoMem that provides typed 32-bit register reads/writes.
#[derive(Clone)]
pub(crate) struct E1000eRegs {
    io_mem: IoMem,
}

impl E1000eRegs {
    /// Creates a new register accessor from an IoMem region.
    pub fn new(io_mem: IoMem) -> Self {
        Self { io_mem }
    }

    /// Returns a reference to the underlying IoMem.
    pub fn io_mem(&self) -> &IoMem {
        &self.io_mem
    }

    /// Reads a 32-bit register at the given byte offset.
    #[inline]
    pub fn read(&self, offset: usize) -> u32 {
        self.io_mem.read_once::<u32>(offset).unwrap()
    }

    /// Writes a 32-bit value to the register at the given byte offset.
    #[inline]
    pub fn write(&self, offset: usize, value: u32) {
        self.io_mem.write_once::<u32>(offset, &value).unwrap();
    }
}
