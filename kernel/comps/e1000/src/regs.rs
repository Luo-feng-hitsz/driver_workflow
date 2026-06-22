// SPDX-License-Identifier: MPL-2.0

//! E1000 register offset constants, bit-field masks, and MMIO accessor helpers.
//! Translated from Linux e1000_hw.h / e1000_osdep.h for the 82540EM (and >= 82543).

#![allow(dead_code)]

use ostd::io::IoMem;
use ostd::mm::VmIoOnce;

// ============================================================================
// Register offsets (82543+ standard layout, NOT 82542 remapped)
// ============================================================================

// Device Control
pub const CTRL: usize = 0x00000;
pub const CTRL_DUP: usize = 0x00004;
pub const STATUS: usize = 0x00008;
pub const EECD: usize = 0x00010;
pub const EERD: usize = 0x00014;
pub const CTRL_EXT: usize = 0x00018;
pub const FLA: usize = 0x0001C;
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
pub const FCTTV: usize = 0x00170;
pub const TXCW: usize = 0x00178;
pub const RXCW: usize = 0x00180;

// Transmit
pub const TCTL: usize = 0x00400;
pub const TCTL_EXT: usize = 0x00404;
pub const TIPG: usize = 0x00410;

// LED
pub const LEDCTL: usize = 0x00E00;

// Packet Buffer Allocation
pub const PBA: usize = 0x01000;

// Flow Control Thresholds
pub const FCRTL: usize = 0x02160;
pub const FCRTH: usize = 0x02168;

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

// Statistics
pub const CRCERRS: usize = 0x04000;
pub const ALGNERRC: usize = 0x04004;
pub const SYMERRS: usize = 0x04008;
pub const RXERRC: usize = 0x0400C;
pub const MPC: usize = 0x04010;
pub const SCC: usize = 0x04014;
pub const ECOL: usize = 0x04018;
pub const MCC: usize = 0x0401C;
pub const LATECOL: usize = 0x04020;
pub const COLC: usize = 0x04028;
pub const DC: usize = 0x04030;
pub const TNCRS: usize = 0x04034;
pub const CEXTERR: usize = 0x0403C;
pub const RLEC: usize = 0x04040;
pub const XONRXC: usize = 0x04048;
pub const XONTXC: usize = 0x0404C;
pub const XOFFRXC: usize = 0x04050;
pub const XOFFTXC: usize = 0x04054;
pub const FCRUC: usize = 0x04058;
pub const GPRC: usize = 0x04074;
pub const BPRC: usize = 0x04078;
pub const MPRC: usize = 0x0407C;
pub const GPTC: usize = 0x04080;
pub const GORCL: usize = 0x04088;
pub const GORCH: usize = 0x0408C;
pub const GOTCL: usize = 0x04090;
pub const GOTCH: usize = 0x04094;
pub const RNBC: usize = 0x040A0;
pub const RUC: usize = 0x040A4;
pub const RFC: usize = 0x040A8;
pub const ROC: usize = 0x040AC;
pub const RJC: usize = 0x040B0;
pub const TORL: usize = 0x040C0;
pub const TORH: usize = 0x040C4;
pub const TOTL: usize = 0x040C8;
pub const TOTH: usize = 0x040CC;
pub const TPR: usize = 0x040D0;
pub const TPT: usize = 0x040D4;
pub const MPTC: usize = 0x040F0;
pub const BPTC: usize = 0x040F4;
pub const TSCTC: usize = 0x040F8;
pub const TSCTFC: usize = 0x040FC;

// RX Checksum Control
pub const RXCSUM: usize = 0x05000;

// Multicast Table Array
pub const MTA: usize = 0x05200;

// Receive Address (RA) registers
pub const RA: usize = 0x05400;

// VLAN Filter Table Array
pub const VFTA: usize = 0x05600;

// Management
pub const MANC: usize = 0x05820;

// ============================================================================
// CTRL register bits
// ============================================================================
pub const CTRL_FD: u32 = 0x00000001;
pub const CTRL_LRST: u32 = 0x00000008;
pub const CTRL_ASDE: u32 = 0x00000020;
pub const CTRL_SLU: u32 = 0x00000040;
pub const CTRL_ILOS: u32 = 0x00000080;
pub const CTRL_SPD_SEL: u32 = 0x00000300;
pub const CTRL_SPD_10: u32 = 0x00000000;
pub const CTRL_SPD_100: u32 = 0x00000100;
pub const CTRL_SPD_1000: u32 = 0x00000200;
pub const CTRL_FRCSPD: u32 = 0x00000800;
pub const CTRL_FRCDPX: u32 = 0x00001000;
pub const CTRL_SWDPIN0: u32 = 0x00040000;
pub const CTRL_SWDPIN1: u32 = 0x00080000;
pub const CTRL_SWDPIO0: u32 = 0x00400000;
pub const CTRL_SWDPIO1: u32 = 0x00800000;
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
pub const STATUS_TXOFF: u32 = 0x00000010;
pub const STATUS_TBIMODE: u32 = 0x00000020;
pub const STATUS_SPEED_MASK: u32 = 0x000000C0;
pub const STATUS_SPEED_10: u32 = 0x00000000;
pub const STATUS_SPEED_100: u32 = 0x00000040;
pub const STATUS_SPEED_1000: u32 = 0x00000080;

// ============================================================================
// EECD register bits (EEPROM/Flash Control)
// ============================================================================
pub const EECD_SK: u32 = 0x00000001;
pub const EECD_CS: u32 = 0x00000002;
pub const EECD_DI: u32 = 0x00000004;
pub const EECD_DO: u32 = 0x00000008;
pub const EECD_FWE_MASK: u32 = 0x00000030;
pub const EECD_FWE_DIS: u32 = 0x00000010;
pub const EECD_FWE_EN: u32 = 0x00000020;
pub const EECD_REQ: u32 = 0x00000040;
pub const EECD_GNT: u32 = 0x00000080;
pub const EECD_PRES: u32 = 0x00000100;
pub const EECD_SIZE: u32 = 0x00000200;
pub const EECD_TYPE: u32 = 0x00002000;

// ============================================================================
// EERD register bits (EEPROM Read)
// ============================================================================
pub const EERD_START: u32 = 0x00000001;
pub const EERD_DONE: u32 = 0x00000010;
pub const EERD_ADDR_SHIFT: u32 = 8;
pub const EERD_DATA_SHIFT: u32 = 16;
pub const EERD_DATA_MASK: u32 = 0xFFFF0000;

// ============================================================================
// MDIC register bits (MDI Control)
// ============================================================================
pub const MDIC_DATA_MASK: u32 = 0x0000FFFF;
pub const MDIC_REG_MASK: u32 = 0x001F0000;
pub const MDIC_REG_SHIFT: u32 = 16;
pub const MDIC_PHY_MASK: u32 = 0x03E00000;
pub const MDIC_PHY_SHIFT: u32 = 21;
pub const MDIC_OP_WRITE: u32 = 0x04000000;
pub const MDIC_OP_READ: u32 = 0x08000000;
pub const MDIC_READY: u32 = 0x10000000;
pub const MDIC_ERROR: u32 = 0x40000000;

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
pub const ICR_MDAC: u32 = 0x00000200;
pub const ICR_INT_ASSERTED: u32 = 0x80000000;

/// Standard IMS enable mask for the e1000 (RXT0 | TXDW | RXDMT0 | RXSEQ | LSC)
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
pub const RCTL_LBM_NO: u32 = 0x00000000;
pub const RCTL_RDMTS_HALF: u32 = 0x00000000;
pub const RCTL_MO_SHIFT: u32 = 12;
pub const RCTL_BAM: u32 = 0x00008000;
pub const RCTL_SZ_2048: u32 = 0x00000000;
pub const RCTL_SZ_4096: u32 = 0x00030000;
pub const RCTL_VFE: u32 = 0x00040000;
pub const RCTL_BSEX: u32 = 0x02000000;
pub const RCTL_SECRC: u32 = 0x04000000;

// ============================================================================
// Transmit Control (TCTL) bits
// ============================================================================
pub const TCTL_EN: u32 = 0x00000002;
pub const TCTL_PSP: u32 = 0x00000008;
pub const TCTL_CT_SHIFT: u32 = 4;
pub const TCTL_COLD_SHIFT: u32 = 12;
pub const TCTL_RTLC: u32 = 0x01000000;

/// Collision threshold for full duplex
pub const COLLISION_THRESHOLD: u32 = 15;
/// Collision distance for full duplex (64-byte times)
pub const COLLISION_DISTANCE_FD: u32 = 64;
/// Collision distance for gigabit half duplex
pub const COLLISION_DISTANCE_HD: u32 = 512;

// ============================================================================
// Transmit Inter-Packet Gap (TIPG)
// ============================================================================
pub const TIPG_IPGT_COPPER: u32 = 8;
pub const TIPG_IPGR1_SHIFT: u32 = 10;
pub const TIPG_IPGR1: u32 = 8;
pub const TIPG_IPGR2_SHIFT: u32 = 20;
pub const TIPG_IPGR2: u32 = 6;

// ============================================================================
// TX Descriptor bits
// ============================================================================
pub const TXD_CMD_EOP: u8 = 0x01;
pub const TXD_CMD_IFCS: u8 = 0x02;
pub const TXD_CMD_RS: u8 = 0x08;
pub const TXD_CMD_DEXT: u8 = 0x20;
pub const TXD_STAT_DD: u8 = 0x01;

// ============================================================================
// RX Descriptor status bits
// ============================================================================
pub const RXD_STAT_DD: u8 = 0x01;
pub const RXD_STAT_EOP: u8 = 0x02;
pub const RXD_STAT_IXSM: u8 = 0x04;
pub const RXD_STAT_VP: u8 = 0x08;
pub const RXD_STAT_TCPCS: u8 = 0x20;
pub const RXD_STAT_IPCS: u8 = 0x40;

// RX Descriptor error bits
pub const RXD_ERR_CE: u8 = 0x01;
pub const RXD_ERR_SE: u8 = 0x02;
pub const RXD_ERR_SEQ: u8 = 0x04;
pub const RXD_ERR_CXE: u8 = 0x10;
pub const RXD_ERR_TCPE: u8 = 0x20;
pub const RXD_ERR_IPE: u8 = 0x40;
pub const RXD_ERR_RXE: u8 = 0x80;

/// Frame error mask
pub const RXD_ERR_FRAME_ERR_MASK: u8 =
    RXD_ERR_CE | RXD_ERR_SE | RXD_ERR_SEQ | RXD_ERR_CXE | RXD_ERR_RXE;

// ============================================================================
// RXCSUM register bits
// ============================================================================
pub const RXCSUM_IPOFL: u32 = 0x00000100;
pub const RXCSUM_TUOFL: u32 = 0x00000200;

// ============================================================================
// TXDCTL register bits
// ============================================================================
pub const TXDCTL_FULL_TX_DESC_WB: u32 = 0x01010000;

// ============================================================================
// Receive Address High
// ============================================================================
pub const RAH_AV: u32 = 0x80000000;

// ============================================================================
// EEPROM offsets
// ============================================================================
pub const EEPROM_ENET_ADDR: u16 = 0x00;
pub const EEPROM_INIT_CTRL_1: u16 = 0x0A;
pub const EEPROM_INIT_CTRL_2: u16 = 0x0F;
pub const EEPROM_CHECKSUM_REG: u16 = 0x3F;
pub const EEPROM_CHECKSUM: u16 = 0xBABA;

// ============================================================================
// PHY registers (M88E1000)
// ============================================================================
pub const PHY_CTRL: u32 = 0x00;
pub const PHY_STATUS: u32 = 0x01;
pub const PHY_ID1: u32 = 0x02;
pub const PHY_ID2: u32 = 0x03;
pub const PHY_AUTONEG_ADV: u32 = 0x04;
pub const PHY_LP_ABILITY: u32 = 0x05;
pub const PHY_AUTONEG_EXP: u32 = 0x06;
pub const PHY_1000T_CTRL: u32 = 0x09;
pub const PHY_1000T_STATUS: u32 = 0x0A;

// M88E1000 specific PHY registers
pub const M88E1000_PHY_SPEC_CTRL: u32 = 0x10;
pub const M88E1000_PHY_SPEC_STATUS: u32 = 0x11;
pub const M88E1000_EXT_PHY_SPEC_CTRL: u32 = 0x14;

// PHY Control register bits
pub const MII_CR_FULL_DUPLEX: u16 = 0x0100;
pub const MII_CR_RESTART_AUTO_NEG: u16 = 0x0200;
pub const MII_CR_AUTO_NEG_EN: u16 = 0x1000;
pub const MII_CR_SPEED_SELECT_MSB: u16 = 0x0040;
pub const MII_CR_SPEED_SELECT_LSB: u16 = 0x2000;
pub const MII_CR_SPEED_1000: u16 = 0x0040;
pub const MII_CR_SPEED_100: u16 = 0x2000;
pub const MII_CR_SPEED_10: u16 = 0x0000;
pub const MII_CR_RESET: u16 = 0x8000;

// PHY Status register bits
pub const MII_SR_LINK_STATUS: u16 = 0x0004;
pub const MII_SR_AUTONEG_COMPLETE: u16 = 0x0020;

// Autoneg advertisement register bits
pub const NWAY_AR_10T_HD_CAPS: u16 = 0x0020;
pub const NWAY_AR_10T_FD_CAPS: u16 = 0x0040;
pub const NWAY_AR_100TX_HD_CAPS: u16 = 0x0080;
pub const NWAY_AR_100TX_FD_CAPS: u16 = 0x0100;
pub const NWAY_AR_PAUSE: u16 = 0x0400;
pub const NWAY_AR_ASM_DIR: u16 = 0x0800;

// 1000BASE-T Control register bits
pub const CR_1000T_HD_CAPS: u16 = 0x0100;
pub const CR_1000T_FD_CAPS: u16 = 0x0200;
pub const CR_1000T_MS_VALUE: u16 = 0x0800;
pub const CR_1000T_MS_ENABLE: u16 = 0x1000;

// M88E1000 PHY-specific Control register bits
pub const M88E1000_PSCR_POLARITY_REVERSAL: u16 = 0x0002;
pub const M88E1000_PSCR_MDI_MANUAL_MODE: u16 = 0x0000;
pub const M88E1000_PSCR_MDIX_MANUAL_MODE: u16 = 0x0020;
pub const M88E1000_PSCR_AUTO_X_1000T: u16 = 0x0040;
pub const M88E1000_PSCR_AUTO_X_MODE: u16 = 0x0060;
pub const M88E1000_PSCR_ASSERT_CRS_ON_TX: u16 = 0x0800;

// M88E1000 PHY-specific Status register bits
pub const M88E1000_PSSR_SPEED_MASK: u16 = 0xC000;
pub const M88E1000_PSSR_1000MBS: u16 = 0x8000;
pub const M88E1000_PSSR_100MBS: u16 = 0x4000;
pub const M88E1000_PSSR_DPLX: u16 = 0x2000;
pub const M88E1000_PSSR_LINK: u16 = 0x0400;
pub const M88E1000_PSSR_CABLE_LENGTH_MASK: u16 = 0x0380;
pub const M88E1000_PSSR_CABLE_LENGTH_SHIFT: u16 = 7;
pub const M88E1000_PSSR_REV_POLARITY: u16 = 0x0002;
pub const M88E1000_PSSR_DOWNSHIFT: u16 = 0x0020;
pub const M88E1000_PSSR_MDIX: u16 = 0x0040;

// M88 PHY ID
pub const M88E1000_I_PHY_ID: u32 = 0x01410C50;
pub const M88E1000_E_PHY_ID: u32 = 0x01410C60;
pub const PHY_REVISION_MASK: u32 = 0x0000000F;

// Speeds
pub const SPEED_10: u16 = 10;
pub const SPEED_100: u16 = 100;
pub const SPEED_1000: u16 = 1000;
pub const HALF_DUPLEX: u16 = 1;
pub const FULL_DUPLEX: u16 = 2;

// Flow Control address and type
pub const FC_DEFAULT_HI_THRESH: u16 = 0x8808;
pub const FC_DEFAULT_LO_THRESH: u16 = 0x8808;
pub const FLOW_CONTROL_ADDRESS_LOW: u32 = 0x00C28001;
pub const FLOW_CONTROL_ADDRESS_HIGH: u32 = 0x00000100;
pub const FLOW_CONTROL_TYPE: u32 = 0x8808;

// Number of Receive Address entries
pub const RAR_ENTRIES: usize = 15;
pub const NUM_MTA_REGISTERS: usize = 128;
pub const VLAN_FILTER_TBL_SIZE: usize = 128;

// MAC decode size (BAR0 size)
pub const MAC_DECODE_SIZE: usize = 128 * 1024;

// ============================================================================
// MMIO accessor wrapper
// ============================================================================

/// A thin wrapper over IoMem that provides typed 32-bit register reads/writes.
#[derive(Clone)]
pub struct E1000Regs {
    io_mem: IoMem,
}

impl E1000Regs {
    /// Creates a new register accessor from an IoMem region.
    pub fn new(io_mem: IoMem) -> Self {
        Self { io_mem }
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

    /// Performs a read-modify-write: sets the bits in `set` and clears the bits in `clear`.
    #[inline]
    pub fn set_clear(&self, offset: usize, set: u32, clear: u32) {
        let val = self.read(offset);
        self.write(offset, (val | set) & !clear);
    }
}
