// SPDX-License-Identifier: MPL-2.0

//! PHY detection, register access via MDIO, autonegotiation setup,
//! PHY reset, speed/duplex configuration for the M88E1000 PHY
//! found on the 82540EM.
//! Translated from e1000_hw.c PHY-related functions.

use crate::hw::E1000Hw;
use crate::regs::*;

// ============================================================================
// PHY Register Access via MDIC
// ============================================================================

/// Reads a PHY register via the MDIC register (82540EM path).
pub fn read_phy_reg(hw: &E1000Hw, reg_addr: u32) -> Result<u16, &'static str> {
    let mdic = ((reg_addr << MDIC_REG_SHIFT) & MDIC_REG_MASK)
        | ((hw.phy_addr << MDIC_PHY_SHIFT) & MDIC_PHY_MASK)
        | MDIC_OP_READ;

    hw.regs.write(MDIC, mdic);

    // Poll for completion (up to 64 iterations ~ 64us)
    for _ in 0..64 {
        spin_delay_us(1);
        let val = hw.regs.read(MDIC);
        if val & MDIC_READY != 0 {
            if val & MDIC_ERROR != 0 {
                return Err("MDIC read error");
            }
            return Ok((val & MDIC_DATA_MASK) as u16);
        }
    }

    Err("MDIC read timeout")
}

/// Writes a PHY register via the MDIC register (82540EM path).
pub fn write_phy_reg(hw: &E1000Hw, reg_addr: u32, data: u16) -> Result<(), &'static str> {
    let mdic = (data as u32)
        | ((reg_addr << MDIC_REG_SHIFT) & MDIC_REG_MASK)
        | ((hw.phy_addr << MDIC_PHY_SHIFT) & MDIC_PHY_MASK)
        | MDIC_OP_WRITE;

    hw.regs.write(MDIC, mdic);

    // Poll for completion
    for _ in 0..64 {
        spin_delay_us(1);
        let val = hw.regs.read(MDIC);
        if val & MDIC_READY != 0 {
            if val & MDIC_ERROR != 0 {
                return Err("MDIC write error");
            }
            return Ok(());
        }
    }

    Err("MDIC write timeout")
}

// ============================================================================
// PHY Detection and Reset
// ============================================================================

/// Detects and identifies the PHY by reading PHY_ID1 and PHY_ID2.
pub fn detect_phy(hw: &mut E1000Hw) -> Result<(), &'static str> {
    let id1 = read_phy_reg(hw, PHY_ID1)? as u32;
    let id2 = read_phy_reg(hw, PHY_ID2)? as u32;
    hw.phy_id = (id1 << 16) | (id2 & 0xFFF0);
    Ok(())
}

/// Performs a hardware reset of the PHY via the CTRL register.
pub fn phy_hw_reset(hw: &E1000Hw) -> Result<(), &'static str> {
    let ctrl = hw.regs.read(CTRL);
    hw.regs.write(CTRL, ctrl | CTRL_PHY_RST);
    // Hold reset for 10us
    spin_delay_us(10);
    hw.regs.write(CTRL, ctrl);
    // Wait 150us for PHY to recover
    spin_delay_us(150);
    Ok(())
}

/// Performs a software reset of the PHY via MII control register.
pub fn phy_reset(hw: &E1000Hw) -> Result<(), &'static str> {
    let mut ctrl = read_phy_reg(hw, PHY_CTRL)?;
    ctrl |= MII_CR_RESET;
    write_phy_reg(hw, PHY_CTRL, ctrl)?;
    // Wait for reset to complete (up to 1ms)
    spin_delay_us(1000);
    Ok(())
}

// ============================================================================
// Copper Link Setup (M88 PHY)
// ============================================================================

/// Sets up the copper (M88 PHY) link for the 82540EM.
/// Configures autoneg or forced speed/duplex.
pub fn setup_copper_link(hw: &mut E1000Hw) -> Result<(), &'static str> {
    // Detect PHY
    detect_phy(hw)?;

    // Reset PHY
    phy_reset(hw)?;

    // Configure M88 PHY-specific control register
    let mut phy_data = read_phy_reg(hw, M88E1000_PHY_SPEC_CTRL)?;
    // Enable CRS on TX, auto crossover
    phy_data |= M88E1000_PSCR_ASSERT_CRS_ON_TX;
    // Set auto crossover mode
    phy_data &= !M88E1000_PSCR_AUTO_X_MODE;
    phy_data |= M88E1000_PSCR_AUTO_X_1000T;
    // Disable polarity reversal
    phy_data &= !M88E1000_PSCR_POLARITY_REVERSAL;
    write_phy_reg(hw, M88E1000_PHY_SPEC_CTRL, phy_data)?;

    if hw.autoneg {
        setup_autoneg(hw)?;
    } else {
        setup_forced_link(hw)?;
    }

    // Force link up in the CTRL register
    let mut ctrl = hw.regs.read(CTRL);
    ctrl |= CTRL_SLU;
    // For autoneg, also set ASDE
    if hw.autoneg {
        ctrl |= CTRL_ASDE;
        ctrl &= !(CTRL_FRCSPD | CTRL_FRCDPX);
    }
    hw.regs.write(CTRL, ctrl);

    // Configure collision distance
    hw.config_collision_dist();

    Ok(())
}

// ============================================================================
// Autonegotiation
// ============================================================================

/// Sets up PHY autonegotiation advertisement registers.
pub fn setup_autoneg(hw: &E1000Hw) -> Result<(), &'static str> {
    // Read current autoneg advertisement
    let mut mii_autoneg_adv = read_phy_reg(hw, PHY_AUTONEG_ADV)?;
    let mut mii_1000t_ctrl = read_phy_reg(hw, PHY_1000T_CTRL)?;

    // Clear and set advertised speeds
    mii_autoneg_adv &= !(NWAY_AR_100TX_FD_CAPS
        | NWAY_AR_100TX_HD_CAPS
        | NWAY_AR_10T_FD_CAPS
        | NWAY_AR_10T_HD_CAPS);
    mii_1000t_ctrl &= !(CR_1000T_HD_CAPS | CR_1000T_FD_CAPS);

    let advertised = hw.autoneg_advertised;

    // Set 10Mbps capabilities
    if advertised & 0x01 != 0 {
        mii_autoneg_adv |= NWAY_AR_10T_HD_CAPS;
    }
    if advertised & 0x02 != 0 {
        mii_autoneg_adv |= NWAY_AR_10T_FD_CAPS;
    }
    // Set 100Mbps capabilities
    if advertised & 0x04 != 0 {
        mii_autoneg_adv |= NWAY_AR_100TX_HD_CAPS;
    }
    if advertised & 0x08 != 0 {
        mii_autoneg_adv |= NWAY_AR_100TX_FD_CAPS;
    }
    // Set 1000Mbps capabilities
    if advertised & 0x10 != 0 {
        mii_1000t_ctrl |= CR_1000T_HD_CAPS;
    }
    if advertised & 0x20 != 0 {
        mii_1000t_ctrl |= CR_1000T_FD_CAPS;
    }

    // Set flow control advertisement
    match hw.fc {
        crate::hw::FlowControlMode::None => {
            mii_autoneg_adv &= !(NWAY_AR_PAUSE | NWAY_AR_ASM_DIR);
        }
        crate::hw::FlowControlMode::RxPause => {
            mii_autoneg_adv |= NWAY_AR_PAUSE | NWAY_AR_ASM_DIR;
        }
        crate::hw::FlowControlMode::TxPause => {
            mii_autoneg_adv |= NWAY_AR_ASM_DIR;
            mii_autoneg_adv &= !NWAY_AR_PAUSE;
        }
        crate::hw::FlowControlMode::Full => {
            mii_autoneg_adv |= NWAY_AR_PAUSE | NWAY_AR_ASM_DIR;
        }
    }

    write_phy_reg(hw, PHY_AUTONEG_ADV, mii_autoneg_adv)?;
    write_phy_reg(hw, PHY_1000T_CTRL, mii_1000t_ctrl)?;

    // Restart autoneg
    let mut mii_ctrl = read_phy_reg(hw, PHY_CTRL)?;
    mii_ctrl |= MII_CR_AUTO_NEG_EN | MII_CR_RESTART_AUTO_NEG;
    write_phy_reg(hw, PHY_CTRL, mii_ctrl)?;

    Ok(())
}

/// Sets up forced speed/duplex mode (no autoneg).
fn setup_forced_link(hw: &E1000Hw) -> Result<(), &'static str> {
    let mut mii_ctrl = read_phy_reg(hw, PHY_CTRL)?;
    mii_ctrl &= !MII_CR_AUTO_NEG_EN;

    // Set speed
    mii_ctrl &= !(MII_CR_SPEED_SELECT_MSB | MII_CR_SPEED_SELECT_LSB);
    match hw.forced_speed_duplex & 0xF0 {
        0x10 => {} // 10 Mbps - both bits 0
        0x20 => {
            mii_ctrl |= MII_CR_SPEED_100;
        }
        _ => {
            mii_ctrl |= MII_CR_SPEED_1000;
        }
    }

    // Set duplex
    if hw.forced_speed_duplex & 0x01 != 0 {
        mii_ctrl |= MII_CR_FULL_DUPLEX;
    } else {
        mii_ctrl &= !MII_CR_FULL_DUPLEX;
    }

    write_phy_reg(hw, PHY_CTRL, mii_ctrl)?;
    Ok(())
}

// ============================================================================
// PHY Info
// ============================================================================

/// Reads the current PHY link status, speed, and duplex from M88 PHY-specific
/// status register.
pub fn get_phy_info(hw: &E1000Hw) -> Result<(bool, u16, u16), &'static str> {
    let phy_status = read_phy_reg(hw, M88E1000_PHY_SPEC_STATUS)?;

    let link = (phy_status & M88E1000_PSSR_LINK) != 0;

    let speed = match phy_status & M88E1000_PSSR_SPEED_MASK {
        M88E1000_PSSR_1000MBS => SPEED_1000,
        M88E1000_PSSR_100MBS => SPEED_100,
        _ => SPEED_10,
    };

    let duplex = if (phy_status & M88E1000_PSSR_DPLX) != 0 {
        FULL_DUPLEX
    } else {
        HALF_DUPLEX
    };

    Ok((link, speed, duplex))
}

/// Powers up the PHY by clearing the power-down bit in MII control register.
pub fn power_up_phy(hw: &E1000Hw) -> Result<(), &'static str> {
    let mut mii_ctrl = read_phy_reg(hw, PHY_CTRL)?;
    mii_ctrl &= !0x0800; // Clear power-down bit
    write_phy_reg(hw, PHY_CTRL, mii_ctrl)?;
    Ok(())
}

// ============================================================================
// Helpers
// ============================================================================

fn spin_delay_us(us: u32) {
    for _ in 0..(us as u64 * 100) {
        core::hint::spin_loop();
    }
}
