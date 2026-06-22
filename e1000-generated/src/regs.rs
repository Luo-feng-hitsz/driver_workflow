// SPDX-License-Identifier: MPL-2.0

//! Intel 82540EM (e1000) register definitions.
//!
//! Only registers relevant to the 82540EM variant are included.
//! Reference: Intel 82540EM Software Developer's Manual.

use bitflags::bitflags;

// =============================================================================
// PCI Identification
// =============================================================================

/// Intel vendor ID.
pub const INTEL_VENDOR_ID: u16 = 0x8086;

/// Supported device IDs for 82540EM and close variants.
pub const SUPPORTED_DEVICE_IDS: &[u16] = &[
    0x100E, // 82540EM (QEMU default)
    0x100F, // 82545EM Copper
    0x1015, // 82540EM LOM
    0x1016, // 82540EP LOM
    0x1017, // 82540EP
    0x101E, // 82540EP LP
];

// =============================================================================
// Register Offsets (MMIO from BAR0)
// =============================================================================

pub const REG_CTRL: usize = 0x00000; // Device Control - RW
pub const REG_STATUS: usize = 0x00008; // Device Status - RO
pub const REG_EECD: usize = 0x00010; // EEPROM/Flash Control - RW
pub const REG_EERD: usize = 0x00014; // EEPROM Read - RW
pub const REG_CTRL_EXT: usize = 0x00018; // Extended Device Control - RW
pub const REG_FCAL: usize = 0x00028; // Flow Control Address Low - RW
pub const REG_FCAH: usize = 0x0002C; // Flow Control Address High - RW
pub const REG_FCT: usize = 0x00030; // Flow Control Type - RW
pub const REG_VET: usize = 0x00038; // VLAN Ether Type - RW
pub const REG_ICR: usize = 0x000C0; // Interrupt Cause Read - R/clr
pub const REG_ITR: usize = 0x000C4; // Interrupt Throttling Rate - RW
pub const REG_ICS: usize = 0x000C8; // Interrupt Cause Set - WO
pub const REG_IMS: usize = 0x000D0; // Interrupt Mask Set - RW
pub const REG_IMC: usize = 0x000D8; // Interrupt Mask Clear - WO
pub const REG_RCTL: usize = 0x00100; // Receive Control - RW
pub const REG_FCTTV: usize = 0x00170; // Flow Control TX Timer Value - RW
pub const REG_TXCW: usize = 0x00178; // TX Configuration Word - RW
pub const REG_RXCW: usize = 0x00180; // RX Configuration Word - RO
pub const REG_TCTL: usize = 0x00400; // Transmit Control - RW
pub const REG_TIPG: usize = 0x00410; // TX Inter-Packet Gap - RW
pub const REG_LEDCTL: usize = 0x00E00; // LED Control - RW
pub const REG_PBA: usize = 0x01000; // Packet Buffer Allocation - RW
pub const REG_FCRTL: usize = 0x02160; // Flow Control RX Threshold Low - RW
pub const REG_FCRTH: usize = 0x02168; // Flow Control RX Threshold High - RW
pub const REG_RDTR: usize = 0x02820; // RX Delay Timer - RW
pub const REG_RADV: usize = 0x0282C; // RX Interrupt Absolute Delay - RW

// RX descriptor ring registers (queue 0)
pub const REG_RDBAL: usize = 0x02800; // RX Descriptor Base Address Low
pub const REG_RDBAH: usize = 0x02804; // RX Descriptor Base Address High
pub const REG_RDLEN: usize = 0x02808; // RX Descriptor Length
pub const REG_RDH: usize = 0x02810; // RX Descriptor Head
pub const REG_RDT: usize = 0x02818; // RX Descriptor Tail

// TX descriptor ring registers (queue 0)
pub const REG_TDBAL: usize = 0x03800; // TX Descriptor Base Address Low
pub const REG_TDBAH: usize = 0x03804; // TX Descriptor Base Address High
pub const REG_TDLEN: usize = 0x03808; // TX Descriptor Length
pub const REG_TDH: usize = 0x03810; // TX Descriptor Head
pub const REG_TDT: usize = 0x03818; // TX Descriptor Tail
pub const REG_TIDV: usize = 0x03820; // TX Interrupt Delay Value - RW
pub const REG_TADV: usize = 0x0382C; // TX Interrupt Absolute Delay - RW

// Multicast Table Array (128 entries x 4 bytes)
pub const REG_MTA: usize = 0x05200;
pub const MTA_ENTRIES: usize = 128;

// Receive Address registers
pub const REG_RAL0: usize = 0x05400; // Receive Address Low (index 0)
pub const REG_RAH0: usize = 0x05404; // Receive Address High (index 0)

// =============================================================================
// CTRL Register Bits (Device Control)
// =============================================================================

bitflags! {
    pub struct Ctrl: u32 {
        const FD       = 1 << 0;   // Full Duplex
        const LRST     = 1 << 3;   // Link Reset
        const ASDE     = 1 << 5;   // Auto-Speed Detection Enable
        const SLU      = 1 << 6;   // Set Link Up
        const ILOS     = 1 << 7;   // Invert Loss-Of-Signal
        const SPD_10   = 0 << 8;   // Speed 10 Mb/s
        const SPD_100  = 1 << 8;   // Speed 100 Mb/s
        const SPD_1000 = 1 << 9;   // Speed 1000 Mb/s
        const FRCSPD   = 1 << 11;  // Force Speed
        const FRCDPX   = 1 << 12;  // Force Duplex
        const RST      = 1 << 26;  // Device Reset
        const RFCE     = 1 << 27;  // Receive Flow Control Enable
        const TFCE     = 1 << 28;  // Transmit Flow Control Enable
        const VME      = 1 << 30;  // VLAN Mode Enable
        const PHY_RST  = 1 << 31;  // PHY Reset
    }
}

// =============================================================================
// STATUS Register Bits (Device Status)
// =============================================================================

bitflags! {
    pub struct Status: u32 {
        const FD         = 1 << 0;  // Full Duplex
        const LU         = 1 << 1;  // Link Up
        const TXOFF      = 1 << 4;  // Transmission Paused
        const SPEED_10   = 0 << 6;  // Speed 10 Mb/s
        const SPEED_100  = 1 << 6;  // Speed 100 Mb/s
        const SPEED_1000 = 1 << 7;  // Speed 1000 Mb/s
    }
}

// =============================================================================
// RCTL Register Bits (Receive Control)
// =============================================================================

bitflags! {
    pub struct Rctl: u32 {
        const EN            = 1 << 1;   // Receiver Enable
        const SBP           = 1 << 2;   // Store Bad Packets
        const UPE           = 1 << 3;   // Unicast Promiscuous Enable
        const MPE           = 1 << 4;   // Multicast Promiscuous Enable
        const LPE           = 1 << 5;   // Long Packet Reception Enable
        const LBM_NONE      = 0 << 6;   // No Loopback
        const RDMTS_HALF    = 0 << 8;   // RX Desc Min Threshold: 1/2
        const RDMTS_QUARTER = 1 << 8;   // RX Desc Min Threshold: 1/4
        const RDMTS_EIGHTH  = 1 << 9;   // RX Desc Min Threshold: 1/8
        const MO_36         = 0 << 12;  // Multicast Offset: bits [47:36]
        const BAM           = 1 << 15;  // Broadcast Accept Mode
        const BSIZE_2048    = 0 << 16;  // Buffer Size 2048
        const BSIZE_1024    = 1 << 16;  // Buffer Size 1024
        const BSIZE_512     = 2 << 16;  // Buffer Size 512
        const BSIZE_256     = 3 << 16;  // Buffer Size 256
        const BSIZE_4096    = 3 << 16;  // Buffer Size 4096 (with BSEX)
        const VFE           = 1 << 18;  // VLAN Filter Enable
        const DPF           = 1 << 22;  // Discard Pause Frames
        const BSEX          = 1 << 25;  // Buffer Size Extension
        const SECRC         = 1 << 26;  // Strip Ethernet CRC
    }
}

// =============================================================================
// TCTL Register Bits (Transmit Control)
// =============================================================================

bitflags! {
    pub struct Tctl: u32 {
        const EN   = 1 << 1;   // Transmit Enable
        const PSP  = 1 << 3;   // Pad Short Packets
        const RTLC = 1 << 24;  // Re-transmit on Late Collision
    }
}

/// Collision Threshold (CT): bits [11:4], recommended value 0x0F.
pub const TCTL_CT_SHIFT: u32 = 4;
pub const TCTL_CT_DEFAULT: u32 = 0x10 << TCTL_CT_SHIFT;

/// Collision Distance (COLD): bits [21:12], recommended 0x40 for full duplex.
pub const TCTL_COLD_SHIFT: u32 = 12;
pub const TCTL_COLD_FD: u32 = 0x40 << TCTL_COLD_SHIFT;

// =============================================================================
// TIPG (Transmit Inter-Packet Gap) - recommended defaults for 82540EM
// =============================================================================

pub const TIPG_IPGT: u32 = 10; // bits [9:0]
pub const TIPG_IPGR1: u32 = 8 << 10; // bits [19:10]
pub const TIPG_IPGR2: u32 = 6 << 20; // bits [29:20]
pub const TIPG_DEFAULT: u32 = TIPG_IPGT | TIPG_IPGR1 | TIPG_IPGR2;

// =============================================================================
// ICR/IMS/IMC Interrupt Bits
// =============================================================================

bitflags! {
    pub struct Interrupt: u32 {
        const TXDW   = 1 << 0;   // Transmit Descriptor Written Back
        const TXQE   = 1 << 1;   // Transmit Queue Empty
        const LSC    = 1 << 2;   // Link Status Change
        const RXSEQ  = 1 << 3;   // Receive Sequence Error
        const RXDMT0 = 1 << 4;   // Receive Descriptor Minimum Threshold
        const RXO    = 1 << 6;   // Receive Overrun
        const RXT0   = 1 << 7;   // Receiver Timer Interrupt
    }
}

// =============================================================================
// EEPROM (EERD Register)
// =============================================================================

pub const EERD_START: u32 = 1 << 0;
pub const EERD_DONE: u32 = 1 << 4;
pub const EERD_ADDR_SHIFT: u32 = 8;
pub const EERD_DATA_SHIFT: u32 = 16;

// =============================================================================
// Receive Address High
// =============================================================================

/// Address Valid bit in RAH register.
pub const RAH_AV: u32 = 1 << 31;

// =============================================================================
// TX Descriptor Command Bits (Legacy)
// =============================================================================

bitflags! {
    pub struct TxCmd: u8 {
        const EOP  = 1 << 0; // End of Packet
        const IFCS = 1 << 1; // Insert FCS (CRC)
        const IC   = 1 << 2; // Insert Checksum
        const RS   = 1 << 3; // Report Status
        const DEXT = 1 << 5; // Descriptor Extension (0 = legacy)
        const VLE  = 1 << 6; // VLAN Packet Enable
        const IDE  = 1 << 7; // Interrupt Delay Enable
    }
}

// TX Descriptor Status Bits
pub const TXD_STAT_DD: u8 = 1 << 0; // Descriptor Done

// =============================================================================
// RX Descriptor Status Bits
// =============================================================================

pub const RXD_STAT_DD: u8 = 1 << 0; // Descriptor Done
pub const RXD_STAT_EOP: u8 = 1 << 1; // End of Packet

// =============================================================================
// Ring Size Constants
// =============================================================================

/// Number of RX descriptors. Must be a multiple of 8.
pub const NUM_RX_DESCS: u16 = 64;
/// Number of TX descriptors. Must be a multiple of 8.
pub const NUM_TX_DESCS: u16 = 64;
/// Size of each RX buffer (matches BSIZE_4096 + BSEX).
pub const RX_BUFFER_SIZE: usize = 4096;
/// Size of each TX buffer.
pub const TX_BUFFER_SIZE: usize = 4096;

// =============================================================================
// Descriptor size
// =============================================================================

/// Size of a single legacy RX or TX descriptor in bytes.
pub const DESC_SIZE: usize = 16;

// =============================================================================
// Flow Control Constants
// =============================================================================

pub const FLOW_CONTROL_ADDRESS_LOW: u32 = 0x00C2_8001;
pub const FLOW_CONTROL_ADDRESS_HIGH: u32 = 0x0000_0100;
pub const FLOW_CONTROL_TYPE: u32 = 0x8808;
