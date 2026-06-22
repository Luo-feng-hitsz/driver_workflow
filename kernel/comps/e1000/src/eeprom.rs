// SPDX-License-Identifier: MPL-2.0

//! EEPROM access for the e1000 82540EM.
//! The 82540EM supports EEPROM reads via the EERD (EEPROM Read) register,
//! which is the simplest and safest access method.
//! Translated from e1000_hw.c EEPROM-related functions.

use crate::hw::E1000Hw;
use crate::regs::*;

// ============================================================================
// EEPROM Read via EERD register
// ============================================================================

/// Reads a single 16-bit word from the EEPROM at the given word address
/// using the hardware EERD register (available on 82540EM and newer).
pub fn read_eeprom(hw: &E1000Hw, offset: u16) -> Result<u16, &'static str> {
    // Write address and start bit to EERD
    let eerd = EERD_START | ((offset as u32) << EERD_ADDR_SHIFT);
    hw.regs.write(EERD, eerd);

    // Poll for EERD_DONE
    for _ in 0..1000 {
        spin_delay_us(5);
        let val = hw.regs.read(EERD);
        if val & EERD_DONE != 0 {
            return Ok(((val & EERD_DATA_MASK) >> EERD_DATA_SHIFT) as u16);
        }
    }

    Err("EEPROM read timeout")
}

/// Reads multiple 16-bit words starting at `offset`.
pub fn read_eeprom_words(
    hw: &E1000Hw,
    offset: u16,
    count: u16,
) -> Result<alloc::vec::Vec<u16>, &'static str> {
    let mut words = alloc::vec::Vec::with_capacity(count as usize);
    for i in 0..count {
        let word = read_eeprom(hw, offset + i)?;
        words.push(word);
    }
    Ok(words)
}

// ============================================================================
// EEPROM Checksum Validation
// ============================================================================

/// Validates the EEPROM checksum.
/// The sum of all 64 EEPROM words should equal 0xBABA.
pub fn validate_eeprom_checksum(hw: &E1000Hw) -> Result<(), &'static str> {
    let mut checksum: u16 = 0;
    for i in 0..64 {
        let word = read_eeprom(hw, i)?;
        checksum = checksum.wrapping_add(word);
    }

    if checksum == EEPROM_CHECKSUM {
        Ok(())
    } else {
        Err("EEPROM checksum invalid")
    }
}

// ============================================================================
// MAC Address from EEPROM
// ============================================================================

/// Reads the MAC address from the EEPROM and stores it in hw.
/// The MAC address is stored in words 0-2 of the EEPROM.
pub fn read_mac_addr_from_eeprom(hw: &mut E1000Hw) -> Result<[u8; 6], &'static str> {
    let mut mac = [0u8; 6];
    for i in 0..3 {
        let word = read_eeprom(hw, EEPROM_ENET_ADDR + i as u16)?;
        mac[i * 2] = (word & 0xFF) as u8;
        mac[i * 2 + 1] = (word >> 8) as u8;
    }
    Ok(mac)
}

// ============================================================================
// SPI/Microwire bit-bang (fallback, not used for EERD-capable 82540EM)
// ============================================================================

/// Raises the EEPROM clock by setting the SK bit in EECD.
fn raise_clock(hw: &E1000Hw, eecd: &mut u32) {
    *eecd |= EECD_SK;
    hw.regs.write(EECD, *eecd);
    spin_delay_us(1);
}

/// Lowers the EEPROM clock by clearing the SK bit in EECD.
fn lower_clock(hw: &E1000Hw, eecd: &mut u32) {
    *eecd &= !EECD_SK;
    hw.regs.write(EECD, *eecd);
    spin_delay_us(1);
}

/// Shifts out `count` bits of `data` to the EEPROM via bit-banging.
fn shift_out_bits(hw: &E1000Hw, data: u16, count: u16) {
    let mut eecd = hw.regs.read(EECD);
    eecd &= !EECD_DO;

    let mut mask = 1u16 << (count - 1);
    while mask != 0 {
        if data & mask != 0 {
            eecd |= EECD_DI;
        } else {
            eecd &= !EECD_DI;
        }
        hw.regs.write(EECD, eecd);
        spin_delay_us(1);
        raise_clock(hw, &mut eecd);
        lower_clock(hw, &mut eecd);
        mask >>= 1;
    }

    eecd &= !EECD_DI;
    hw.regs.write(EECD, eecd);
}

/// Shifts in `count` bits from the EEPROM via bit-banging.
fn shift_in_bits(hw: &E1000Hw, count: u16) -> u16 {
    let mut eecd = hw.regs.read(EECD);
    eecd &= !(EECD_DO | EECD_DI);
    let mut data: u16 = 0;

    for _ in 0..count {
        data <<= 1;
        raise_clock(hw, &mut eecd);
        eecd = hw.regs.read(EECD);
        if eecd & EECD_DO != 0 {
            data |= 1;
        }
        lower_clock(hw, &mut eecd);
    }

    data
}

/// Acquires the EEPROM (sets CS high for microwire, or prepares for SPI).
fn acquire_eeprom(hw: &E1000Hw) {
    let mut eecd = hw.regs.read(EECD);
    eecd |= EECD_CS;
    hw.regs.write(EECD, eecd);
    spin_delay_us(1);
}

/// Releases the EEPROM (deasserts CS).
fn release_eeprom(hw: &E1000Hw) {
    let mut eecd = hw.regs.read(EECD);
    eecd &= !EECD_CS;
    hw.regs.write(EECD, eecd);
    spin_delay_us(1);
}

// ============================================================================
// Internal helpers
// ============================================================================

fn spin_delay_us(us: u32) {
    for _ in 0..(us as u64 * 100) {
        core::hint::spin_loop();
    }
}
