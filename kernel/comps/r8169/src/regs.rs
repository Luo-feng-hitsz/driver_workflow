// SPDX-License-Identifier: MPL-2.0

//! Register offset constants, register content bit flags, descriptor bit flags,
//! and MMIO accessor types for the RTL8168g (r8169) network controller.
//!
//! Translated from: drivers/net/ethernet/realtek/r8169_main.c

use aster_pci::cfg_space::BarAccess;
use ostd::Result;

// ---------------------------------------------------------------------------
// RTL register offsets (enum rtl_registers in C)
// ---------------------------------------------------------------------------

/// Ethernet hardware address bytes 0-3.
pub const MAC0: u16 = 0x00;
/// Ethernet hardware address bytes 4-5.
pub const MAC4: u16 = 0x04;
/// Multicast filter.
pub const MAR0: u16 = 0x08;

pub const COUNTER_ADDR_LOW: u16 = 0x10;
pub const COUNTER_ADDR_HIGH: u16 = 0x14;

pub const TX_DESC_START_ADDR_LOW: u16 = 0x20;
pub const TX_DESC_START_ADDR_HIGH: u16 = 0x24;

pub const CHIP_CMD: u16 = 0x37;
pub const TX_POLL: u16 = 0x38;
pub const INTR_MASK: u16 = 0x3c;
pub const INTR_STATUS: u16 = 0x3e;

pub const TX_CONFIG: u16 = 0x40;
pub const RX_CONFIG: u16 = 0x44;

pub const CFG_9346: u16 = 0x50;
pub const CONFIG0: u16 = 0x51;
pub const CONFIG1: u16 = 0x52;
pub const CONFIG2: u16 = 0x53;
pub const CONFIG3: u16 = 0x54;
pub const CONFIG4: u16 = 0x55;
pub const CONFIG5: u16 = 0x56;

pub const PHYAR: u16 = 0x60;
pub const PHY_STATUS: u16 = 0x6c;

pub const RX_MAX_SIZE: u16 = 0xda;
pub const CPLUS_CMD: u16 = 0xe0;
pub const INTR_MITIGATE: u16 = 0xe2;

pub const RX_DESC_ADDR_LOW: u16 = 0xe4;
pub const RX_DESC_ADDR_HIGH: u16 = 0xe8;

/// MaxTxPacketSize register (8101/8168). Unit of 128 bytes.
pub const MAX_TX_PACKET_SIZE: u16 = 0xec;

pub const FUNC_EVENT: u16 = 0xf0;
pub const FUNC_EVENT_MASK: u16 = 0xf4;
pub const FUNC_PRESET_STATE: u16 = 0xf8;

// ---------------------------------------------------------------------------
// RTL8168/8101 registers (enum rtl8168_8101_registers in C)
// ---------------------------------------------------------------------------

pub const CSIDR: u16 = 0x64;
pub const CSIAR: u16 = 0x68;

pub const CSIAR_FLAG: u32 = 0x8000_0000;
pub const CSIAR_WRITE_CMD: u32 = 0x8000_0000;
pub const CSIAR_BYTE_ENABLE: u32 = 0x0000_f000;
pub const CSIAR_ADDR_MASK: u32 = 0x0000_0fff;

pub const PMCH: u16 = 0x6f;
pub const D3COLD_NO_PLL_DOWN: u8 = 1 << 7;
pub const D3HOT_NO_PLL_DOWN: u8 = 1 << 6;
pub const D3_NO_PLL_DOWN: u8 = (1 << 7) | (1 << 6);

pub const EPHYAR: u16 = 0x80;
pub const EPHYAR_FLAG: u32 = 0x8000_0000;
pub const EPHYAR_WRITE_CMD: u32 = 0x8000_0000;
pub const EPHYAR_REG_MASK: u32 = 0x1f;
pub const EPHYAR_REG_SHIFT: u32 = 16;
pub const EPHYAR_DATA_MASK: u32 = 0xffff;

pub const DLLPR: u16 = 0xd0;
pub const PFM_EN: u8 = 1 << 6;
pub const TX_10M_PS_EN: u8 = 1 << 7;

pub const DBG_REG: u16 = 0xd1;
pub const FIX_NAK_1: u8 = 1 << 4;
pub const FIX_NAK_2: u8 = 1 << 3;

pub const MCU: u16 = 0xd3;
pub const NOW_IS_OOB: u8 = 1 << 7;
pub const TX_EMPTY: u8 = 1 << 5;
pub const RX_EMPTY: u8 = 1 << 4;
pub const RXTX_EMPTY: u8 = TX_EMPTY | RX_EMPTY;
pub const EN_NDP: u8 = 1 << 3;
pub const EN_OOB_RESET: u8 = 1 << 2;
pub const LINK_LIST_RDY: u8 = 1 << 1;

pub const EFUSEAR: u16 = 0xdc;
pub const EFUSEAR_FLAG: u32 = 0x8000_0000;

pub const MISC_1: u16 = 0xf2;
pub const PFM_D3COLD_EN: u8 = 1 << 6;

// ---------------------------------------------------------------------------
// RTL8168 registers (enum rtl8168_registers in C)
// ---------------------------------------------------------------------------

pub const LED_CTRL: u16 = 0x18;
pub const LED_FREQ: u16 = 0x1a;
pub const EEE_LED: u16 = 0x1b;

pub const ERIDR: u16 = 0x70;
pub const ERIAR: u16 = 0x74;

pub const ERIAR_FLAG: u32 = 0x8000_0000;
pub const ERIAR_WRITE_CMD: u32 = 0x8000_0000;
pub const ERIAR_READ_CMD: u32 = 0x0000_0000;
pub const ERIAR_ADDR_BYTE_ALIGN: u32 = 4;
pub const ERIAR_TYPE_SHIFT: u32 = 16;
pub const ERIAR_EXGMAC: u32 = 0x00 << ERIAR_TYPE_SHIFT;
pub const ERIAR_MSIX: u32 = 0x01 << ERIAR_TYPE_SHIFT;
pub const ERIAR_ASF: u32 = 0x02 << ERIAR_TYPE_SHIFT;
pub const ERIAR_OOB: u32 = 0x02 << ERIAR_TYPE_SHIFT;
pub const ERIAR_MASK_SHIFT: u32 = 12;
pub const ERIAR_MASK_0001: u32 = 0x1 << ERIAR_MASK_SHIFT;
pub const ERIAR_MASK_0011: u32 = 0x3 << ERIAR_MASK_SHIFT;
pub const ERIAR_MASK_0100: u32 = 0x4 << ERIAR_MASK_SHIFT;
pub const ERIAR_MASK_0101: u32 = 0x5 << ERIAR_MASK_SHIFT;
pub const ERIAR_MASK_1111: u32 = 0xf << ERIAR_MASK_SHIFT;

pub const OCPDR: u16 = 0xb0;
pub const OCPDR_WRITE_CMD: u32 = 0x8000_0000;
pub const OCPDR_READ_CMD: u32 = 0x0000_0000;
pub const OCPDR_REG_MASK: u32 = 0x7f;
pub const OCPDR_GPHY_REG_SHIFT: u32 = 16;
pub const OCPDR_DATA_MASK: u32 = 0xffff;

pub const OCPAR: u16 = 0xb4;
pub const OCPAR_FLAG: u32 = 0x8000_0000;
pub const OCPAR_GPHY_WRITE_CMD: u32 = 0x8000_f060;
pub const OCPAR_GPHY_READ_CMD: u32 = 0x0000_f060;

pub const GPHY_OCP: u16 = 0xb8;

pub const MISC: u16 = 0xf0;
pub const TXPLA_RST: u32 = 1 << 29;
pub const DISABLE_LAN_EN: u32 = 1 << 23;
pub const PWM_EN: u32 = 1 << 22;
pub const RXDV_GATED_EN: u32 = 1 << 19;
pub const EARLY_TALLY_EN: u32 = 1 << 16;

// ---------------------------------------------------------------------------
// Register content bit flags (enum rtl_register_content in C)
// ---------------------------------------------------------------------------

// Interrupt status bits
pub const SYS_ERR: u16 = 0x8000;
pub const PCS_TIMEOUT: u16 = 0x4000;
pub const SW_INT: u16 = 0x0100;
pub const TX_DESC_UNAVAIL: u16 = 0x0080;
pub const RX_FIFO_OVER: u16 = 0x0040;
pub const LINK_CHG: u16 = 0x0020;
pub const RX_OVERFLOW: u16 = 0x0010;
pub const TX_ERR: u16 = 0x0008;
pub const TX_OK: u16 = 0x0004;
pub const RX_ERR: u16 = 0x0002;
pub const RX_OK: u16 = 0x0001;

// ChipCmd bits
pub const STOP_REQ: u8 = 0x80;
pub const CMD_RESET: u8 = 0x10;
pub const CMD_RX_ENB: u8 = 0x08;
pub const CMD_TX_ENB: u8 = 0x04;
pub const RX_BUF_EMPTY: u8 = 0x01;

// TxPoll register
pub const HPQ: u8 = 0x80;
pub const NPQ: u8 = 0x40;
pub const FS_WINT: u8 = 0x01;

// Cfg9346 bits
pub const CFG_9346_LOCK: u8 = 0x00;
pub const CFG_9346_UNLOCK: u8 = 0xc0;

// RX mode bits
pub const ACCEPT_ERR: u32 = 0x20;
pub const ACCEPT_RUNT: u32 = 0x10;
pub const RX_CONFIG_ACCEPT_ERR_MASK: u32 = 0x30;
pub const ACCEPT_BROADCAST: u32 = 0x08;
pub const ACCEPT_MULTICAST: u32 = 0x04;
pub const ACCEPT_MY_PHYS: u32 = 0x02;
pub const ACCEPT_ALL_PHYS: u32 = 0x01;
pub const RX_CONFIG_ACCEPT_OK_MASK: u32 = 0x0f;
pub const RX_CONFIG_ACCEPT_MASK: u32 = 0x3f;

// TxConfig bits
pub const TX_INTERFRAME_GAP_SHIFT: u32 = 24;
pub const TX_DMA_SHIFT: u32 = 8;

// Config1
pub const LEDS1: u8 = 1 << 7;
pub const LEDS0: u8 = 1 << 6;
pub const SPEED_DOWN: u8 = 1 << 4;
pub const MEMMAP: u8 = 1 << 3;
pub const IOMAP: u8 = 1 << 2;
pub const VPD: u8 = 1 << 1;
pub const PM_ENABLE: u8 = 1 << 0;

// Config2
pub const CLK_REQ_EN: u8 = 1 << 7;
pub const MSI_ENABLE: u8 = 1 << 5;

// Config3
pub const MAGIC_PACKET: u8 = 1 << 5;
pub const LINK_UP: u8 = 1 << 4;
pub const JUMBO_EN0: u8 = 1 << 2;
pub const RDY_TO_L23: u8 = 1 << 1;
pub const BEACON_EN: u8 = 1 << 0;

// Config4
pub const JUMBO_EN1: u8 = 1 << 1;

// Config5
pub const BWF: u8 = 1 << 6;
pub const MWF: u8 = 1 << 5;
pub const UWF: u8 = 1 << 4;
pub const SPI_EN: u8 = 1 << 3;
pub const LAN_WAKE: u8 = 1 << 1;
pub const PME_STATUS: u8 = 1 << 0;
pub const ASPM_EN: u8 = 1 << 0;

// CPlusCmd bits
pub const ENABLE_BIST: u16 = 1 << 15;
pub const EN_ANA_PLL: u16 = 1 << 14;
pub const FORCE_HALF_DUP: u16 = 1 << 12;
pub const FORCE_RXFLOW_EN: u16 = 1 << 11;
pub const FORCE_TXFLOW_EN: u16 = 1 << 10;
pub const PKT_CNTR_DISABLE: u16 = 1 << 7;
pub const RX_VLAN: u16 = 1 << 6;
pub const RX_CHK_SUM: u16 = 1 << 5;
pub const PCIDAC: u16 = 1 << 4;
pub const PCI_MUL_RW: u16 = 1 << 3;
pub const INTT_MASK: u16 = 0x0003;
pub const CPCMD_MASK: u16 = (1 << 13) | RX_VLAN | RX_CHK_SUM | INTT_MASK;

// PHY status
pub const TBI_ENABLE: u8 = 0x80;
pub const TX_FLOW_CTRL: u8 = 0x40;
pub const RX_FLOW_CTRL: u8 = 0x20;
pub const GBPS_F_1000: u8 = 0x10;
pub const MBPS_100: u8 = 0x08;
pub const MBPS_10: u8 = 0x04;
pub const LINK_STATUS_FLAG: u8 = 0x02;
pub const FULL_DUP: u8 = 0x01;

// Counter commands
pub const COUNTER_RESET: u32 = 0x1;
pub const COUNTER_DUMP: u32 = 0x8;

// PME signal (Config2)
pub const PME_SIGNAL: u8 = 1 << 5;

// RxConfig bits
pub const RX128_INT_EN: u32 = 1 << 15;
pub const RX_MULTI_EN: u32 = 1 << 14;
pub const RXCFG_FIFO_SHIFT: u32 = 13;
pub const RX_FIFO_THRESH: u32 = 7 << RXCFG_FIFO_SHIFT;
pub const RX_EARLY_OFF: u32 = 1 << 11;
pub const RXCFG_DMA_SHIFT: u32 = 8;
pub const RX_DMA_BURST: u32 = 7 << RXCFG_DMA_SHIFT;

// TxConfig
pub const TXCFG_AUTO_FIFO: u32 = 1 << 7;
pub const TXCFG_EMPTY: u32 = 1 << 11;

// MaxTxPacketSize values
pub const TX_PACKET_MAX: u8 = (8064 >> 7) as u8;
pub const EARLY_SIZE: u8 = 0x27;

// ---------------------------------------------------------------------------
// Descriptor bit flags
// ---------------------------------------------------------------------------

/// Descriptor is owned by NIC.
pub const DESC_OWN: u32 = 1 << 31;
/// End of descriptor ring.
pub const RING_END: u32 = 1 << 30;
/// First segment of a packet.
pub const FIRST_FRAG: u32 = 1 << 29;
/// Final segment of a packet.
pub const LAST_FRAG: u32 = 1 << 28;

// TX descriptor bits (generic)
pub const TD_LSO: u32 = 1 << 27;
pub const TD_MSS_MAX: u32 = 0x07ff;

// TX VLAN
pub const TX_VLAN_TAG: u32 = 1 << 17;

// TX descriptor bits v2 (8102e, 8168c and beyond -- used by RTL8168g)
pub const TD1_GTSEN_V4: u32 = 1 << 26;
pub const TD1_GTSEN_V6: u32 = 1 << 25;
pub const GTTCPHO_SHIFT: u32 = 18;
pub const GTTCPHO_MAX: u32 = 0x7f;
pub const TCPHO_SHIFT: u32 = 18;
pub const TCPHO_MAX: u32 = 0x3ff;
pub const TD1_MSS_SHIFT: u32 = 18;
pub const TD1_IPV6_CS: u32 = 1 << 28;
pub const TD1_IPV4_CS: u32 = 1 << 29;
pub const TD1_TCP_CS: u32 = 1 << 30;
pub const TD1_UDP_CS: u32 = 1 << 31;

// RX descriptor bits
pub const PID1: u32 = 1 << 18;
pub const PID0: u32 = 1 << 17;
pub const RX_PROTO_UDP: u32 = PID1;
pub const RX_PROTO_TCP: u32 = PID0;
pub const RX_PROTO_IP: u32 = PID1 | PID0;
pub const RX_PROTO_MASK: u32 = RX_PROTO_IP;
pub const IP_FAIL: u32 = 1 << 16;
pub const UDP_FAIL: u32 = 1 << 15;
pub const TCP_FAIL: u32 = 1 << 14;
pub const RX_CS_FAIL_MASK: u32 = IP_FAIL | UDP_FAIL | TCP_FAIL;
pub const RX_VLAN_TAG: u32 = 1 << 16;

// RX status
pub const RX_RWT: u32 = 1 << 22;
pub const RX_RES: u32 = 1 << 21;
pub const RX_RUNT: u32 = 1 << 20;
pub const RX_CRC: u32 = 1 << 19;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const TX_DMA_BURST: u32 = 7; // Maximum PCI burst, '7' is unlimited
pub const INTER_FRAME_GAP: u32 = 0x03; // shortest

pub const R8169_REGS_SIZE: usize = 256;
/// RX buffer size = 16K - 1.
pub const R8169_RX_BUF_SIZE: u32 = 16 * 1024 - 1;

pub const NUM_TX_DESC: usize = 256;
pub const NUM_RX_DESC: usize = 256;

pub const R8169_TX_STOP_THRS: usize = 2; // simplified: no frags support
pub const R8169_TX_START_THRS: usize = 2 * R8169_TX_STOP_THRS;

pub const OCP_STD_PHY_BASE: u32 = 0xa400;

pub const VLAN_ETH_HLEN: usize = 18;
pub const ETH_FCS_LEN: usize = 4;
pub const ETH_ALEN: usize = 6;

pub const JUMBO_9K: usize = 9 * 1024 - VLAN_ETH_HLEN - ETH_FCS_LEN;

// ---------------------------------------------------------------------------
// MMIO accessor wrapper
// ---------------------------------------------------------------------------

/// Safe MMIO accessor wrapping PCI BAR access.
#[derive(Clone, Debug)]
pub struct Mmio {
    bar: BarAccess,
}

impl Mmio {
    /// Creates a new MMIO accessor from a PCI BAR access handle.
    pub fn new(bar: BarAccess) -> Self {
        Self { bar }
    }

    /// Reads an 8-bit value from the given register offset.
    pub fn read8(&self, reg: u16) -> Result<u8> {
        self.bar.read_once::<u8>(reg as usize)
    }

    /// Reads a 16-bit value from the given register offset.
    pub fn read16(&self, reg: u16) -> Result<u16> {
        self.bar.read_once::<u16>(reg as usize)
    }

    /// Reads a 32-bit value from the given register offset.
    pub fn read32(&self, reg: u16) -> Result<u32> {
        self.bar.read_once::<u32>(reg as usize)
    }

    /// Writes an 8-bit value to the given register offset.
    pub fn write8(&self, reg: u16, val: u8) -> Result<()> {
        self.bar.write_once::<u8>(reg as usize, val)
    }

    /// Writes a 16-bit value to the given register offset.
    pub fn write16(&self, reg: u16, val: u16) -> Result<()> {
        self.bar.write_once::<u16>(reg as usize, val)
    }

    /// Writes a 32-bit value to the given register offset.
    pub fn write32(&self, reg: u16, val: u32) -> Result<()> {
        self.bar.write_once::<u32>(reg as usize, val)
    }
}
