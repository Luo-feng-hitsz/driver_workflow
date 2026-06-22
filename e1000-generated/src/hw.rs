// SPDX-License-Identifier: MPL-2.0

//! Hardware operations for the Intel 82540EM.
//!
//! All MMIO access goes through Asterinas `IoMem` (read_once/write_once),
//! never through raw pointers.
//!
//! Includes: device reset, EEPROM read, MAC address read, link setup,
//! interrupt configuration, RX/TX hardware ring programming.

use aster_network::EthernetAddr;
use ostd::{io::IoMem, mm::VmIoOnce};

use crate::regs::*;

// =============================================================================
// Register access helpers
// =============================================================================

/// Reads a 32-bit MMIO register at the given offset.
#[inline]
pub fn read_reg(io_mem: &IoMem, offset: usize) -> u32 {
    io_mem.read_once(offset).unwrap()
}

/// Writes a 32-bit value to an MMIO register at the given offset.
#[inline]
pub fn write_reg(io_mem: &IoMem, offset: usize, value: u32) {
    io_mem.write_once(offset, &value).unwrap();
}

// =============================================================================
// Device Reset
// =============================================================================

/// Performs a full device reset and waits for completion.
///
/// After reset, all interrupts are disabled and pending interrupts are cleared.
pub fn reset_device(io_mem: &IoMem) {
    let ctrl = read_reg(io_mem, REG_CTRL);
    write_reg(io_mem, REG_CTRL, ctrl | Ctrl::RST.bits());

    // Spin until the RST bit self-clears.
    loop {
        if read_reg(io_mem, REG_CTRL) & Ctrl::RST.bits() == 0 {
            break;
        }
        core::hint::spin_loop();
    }

    // Disable all interrupts and clear any pending.
    write_reg(io_mem, REG_IMC, 0xFFFF_FFFF);
    let _ = read_reg(io_mem, REG_ICR);
}

// =============================================================================
// EEPROM Access
// =============================================================================

/// Reads a 16-bit word from the EEPROM at the given word address.
///
/// Uses the EERD register polling method (suitable for 82540EM).
pub fn read_eeprom(io_mem: &IoMem, addr: u8) -> u16 {
    write_reg(
        io_mem,
        REG_EERD,
        EERD_START | (u32::from(addr) << EERD_ADDR_SHIFT),
    );
    loop {
        let val = read_reg(io_mem, REG_EERD);
        if val & EERD_DONE != 0 {
            return ((val >> EERD_DATA_SHIFT) & 0xFFFF) as u16;
        }
        core::hint::spin_loop();
    }
}

// =============================================================================
// MAC Address
// =============================================================================

/// Reads the MAC address from the Receive Address registers or EEPROM.
///
/// First checks RAL0/RAH0 for a valid address. If not present, reads
/// from EEPROM words 0-2 and programs the RA registers.
pub fn read_mac_address(io_mem: &IoMem) -> EthernetAddr {
    let ral: u32 = read_reg(io_mem, REG_RAL0);
    let rah: u32 = read_reg(io_mem, REG_RAH0);

    if rah & RAH_AV != 0 {
        return EthernetAddr([
            (ral & 0xFF) as u8,
            ((ral >> 8) & 0xFF) as u8,
            ((ral >> 16) & 0xFF) as u8,
            ((ral >> 24) & 0xFF) as u8,
            (rah & 0xFF) as u8,
            ((rah >> 8) & 0xFF) as u8,
        ]);
    }

    // Fallback: read from EEPROM
    let mut mac = [0u8; 6];
    for i in 0..3u8 {
        let word = read_eeprom(io_mem, i);
        mac[i as usize * 2] = (word & 0xFF) as u8;
        mac[i as usize * 2 + 1] = ((word >> 8) & 0xFF) as u8;
    }

    // Program the Receive Address registers
    let ral_val = u32::from(mac[0])
        | (u32::from(mac[1]) << 8)
        | (u32::from(mac[2]) << 16)
        | (u32::from(mac[3]) << 24);
    let rah_val = u32::from(mac[4]) | (u32::from(mac[5]) << 8) | RAH_AV;
    write_reg(io_mem, REG_RAL0, ral_val);
    write_reg(io_mem, REG_RAH0, rah_val);

    EthernetAddr(mac)
}

// =============================================================================
// Multicast Table
// =============================================================================

/// Clears the entire Multicast Table Array (128 x 32-bit entries).
pub fn clear_multicast_table(io_mem: &IoMem) {
    for i in 0..MTA_ENTRIES {
        write_reg(io_mem, REG_MTA + i * 4, 0);
    }
}

// =============================================================================
// Link Setup
// =============================================================================

/// Configures the link: sets link up with auto-speed detection.
pub fn setup_link(io_mem: &IoMem) {
    let ctrl = read_reg(io_mem, REG_CTRL);
    write_reg(
        io_mem,
        REG_CTRL,
        ctrl | Ctrl::SLU.bits() | Ctrl::ASDE.bits(),
    );
}

/// Returns true if the link is up (STATUS.LU bit set).
pub fn is_link_up(io_mem: &IoMem) -> bool {
    read_reg(io_mem, REG_STATUS) & Status::LU.bits() != 0
}

// =============================================================================
// Flow Control Setup
// =============================================================================

/// Programs flow control registers with IEEE 802.3 defaults.
pub fn setup_flow_control(io_mem: &IoMem) {
    write_reg(io_mem, REG_FCAL, FLOW_CONTROL_ADDRESS_LOW);
    write_reg(io_mem, REG_FCAH, FLOW_CONTROL_ADDRESS_HIGH);
    write_reg(io_mem, REG_FCT, FLOW_CONTROL_TYPE);
    write_reg(io_mem, REG_FCTTV, 0);
}

// =============================================================================
// Receive Hardware Setup
// =============================================================================

/// Programs the RX descriptor ring base, length, head/tail into hardware,
/// and enables the receiver.
///
/// `ring_dma` is the DMA (physical) address of the descriptor ring memory.
/// `num_descs` is the total number of descriptors.
pub fn setup_rx_hardware(io_mem: &IoMem, ring_dma: u64, num_descs: usize) {
    write_reg(io_mem, REG_RDBAL, ring_dma as u32);
    write_reg(io_mem, REG_RDBAH, (ring_dma >> 32) as u32);
    write_reg(io_mem, REG_RDLEN, (num_descs * DESC_SIZE) as u32);
    write_reg(io_mem, REG_RDH, 0);
    write_reg(io_mem, REG_RDT, (num_descs as u32) - 1);

    // Enable receiver: accept broadcast, 4096-byte buffers, strip CRC
    let rctl = Rctl::EN | Rctl::BAM | Rctl::BSIZE_4096 | Rctl::BSEX | Rctl::SECRC;
    write_reg(io_mem, REG_RCTL, rctl.bits());
}

// =============================================================================
// Transmit Hardware Setup
// =============================================================================

/// Programs the TX descriptor ring base, length, head/tail into hardware,
/// enables the transmitter, and sets the inter-packet gap.
///
/// `ring_dma` is the DMA (physical) address of the descriptor ring memory.
/// `num_descs` is the total number of descriptors.
pub fn setup_tx_hardware(io_mem: &IoMem, ring_dma: u64, num_descs: usize) {
    write_reg(io_mem, REG_TDBAL, ring_dma as u32);
    write_reg(io_mem, REG_TDBAH, (ring_dma >> 32) as u32);
    write_reg(io_mem, REG_TDLEN, (num_descs * DESC_SIZE) as u32);
    write_reg(io_mem, REG_TDH, 0);
    write_reg(io_mem, REG_TDT, 0);

    // Set inter-packet gap to recommended defaults
    write_reg(io_mem, REG_TIPG, TIPG_DEFAULT);

    // Enable transmitter with recommended collision parameters
    let tctl = Tctl::EN.bits() | Tctl::PSP.bits() | TCTL_CT_DEFAULT | TCTL_COLD_FD;
    write_reg(io_mem, REG_TCTL, tctl);
}

// =============================================================================
// Interrupt Control
// =============================================================================

/// Disables all interrupts.
pub fn disable_interrupts(io_mem: &IoMem) {
    write_reg(io_mem, REG_IMC, 0xFFFF_FFFF);
}

/// Enables the standard set of interrupts for normal operation.
pub fn enable_interrupts(io_mem: &IoMem) {
    let ims = Interrupt::RXT0 | Interrupt::TXDW | Interrupt::LSC | Interrupt::RXDMT0;
    write_reg(io_mem, REG_IMS, ims.bits());
}

/// Reads and clears the Interrupt Cause Register.
pub fn read_and_clear_icr(io_mem: &IoMem) -> u32 {
    read_reg(io_mem, REG_ICR)
}
