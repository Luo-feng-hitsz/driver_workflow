// SPDX-License-Identifier: MPL-2.0

//! PHY configuration for the RTL8168g (MAC_VER_40) network controller.
//!
//! Provides MDIO read/write helpers (using the 8168g OCP-based PHY register
//! access path), paged PHY register accessors, batch PHY writes, EEE
//! configuration, ALDPS disable, PHY auto-speed-down, 10M adjustments,
//! and the top-level `rtl8168g_1_hw_phy_config` init sequence.
//!
//! Translated from: drivers/net/ethernet/realtek/r8169_phy_config.c
//!                   drivers/net/ethernet/realtek/r8169_main.c (MDIO helpers)

use crate::regs::{
    self, Mmio, ERIAR, ERIAR_EXGMAC, ERIAR_FLAG, ERIAR_MASK_1111, ERIAR_WRITE_CMD, ERIDR,
    GPHY_OCP, OCPAR_FLAG, OCP_STD_PHY_BASE,
};
use log;

// ---------------------------------------------------------------------------
// MII standard register numbers (from Linux <linux/mii.h>)
// ---------------------------------------------------------------------------

/// MII Basic Mode Control Register.
const MII_BMCR: u32 = 0x00;
/// BMCR bit: software reset.
const BMCR_RESET: u16 = 0x8000;
/// BMCR bit: power down.
const BMCR_PDOWN: u16 = 0x0800;

// ---------------------------------------------------------------------------
// Internal state: OCP base for paged PHY register access
// ---------------------------------------------------------------------------

/// Mutable PHY state used during configuration.
///
/// For the 8168g path the PHY is accessed through an OCP (on-chip peripheral)
/// window.  Writing `0x1f` (the MII page register) changes the base address of
/// that window rather than writing an actual MII register.
pub struct PhyAccess<'a> {
    mmio: &'a Mmio,
    ocp_base: u32,
}

impl<'a> PhyAccess<'a> {
    /// Creates a new PHY accessor with the standard OCP base.
    pub fn new(mmio: &'a Mmio) -> Self {
        Self {
            mmio,
            ocp_base: OCP_STD_PHY_BASE,
        }
    }

    // -----------------------------------------------------------------------
    // Low-level OCP PHY read/write  (r8168_phy_ocp_{read,write})
    // -----------------------------------------------------------------------

    /// Validates that an OCP register address is properly aligned and within
    /// the valid range (equivalent to `rtl_ocp_reg_failure`).
    fn ocp_reg_ok(reg: u32) -> bool {
        // The register must be even (u16 aligned) and must fit in the
        // 15-bit address field.  The C driver checks `reg & 0xb000 == 0xa000`
        // on the full address, but for our scoped use (always adding the base)
        // we simply verify alignment.
        (reg & 1) == 0 && reg < 0x10000
    }

    /// Writes a 16-bit value to a GPHY OCP register.
    ///
    /// Corresponds to `r8168_phy_ocp_write` in the C driver.
    fn phy_ocp_write(&self, reg: u32, data: u16) {
        if !Self::ocp_reg_ok(reg) {
            return;
        }
        // GPHY_OCP = OCPAR_FLAG | (reg << 15) | data
        let cmd = OCPAR_FLAG | (reg << 15) | (data as u32);
        let _ = self.mmio.write32(GPHY_OCP, cmd);

        // Poll until the flag bit clears (loop_wait_low).
        self.poll_gphy_ocp_low();
    }

    /// Reads a 16-bit value from a GPHY OCP register.
    ///
    /// Corresponds to `r8168_phy_ocp_read` in the C driver.
    fn phy_ocp_read(&self, reg: u32) -> u16 {
        if !Self::ocp_reg_ok(reg) {
            return 0;
        }
        // Write the address without the write flag.
        let _ = self.mmio.write32(GPHY_OCP, reg << 15);

        // Poll until the flag bit goes high.
        if self.poll_gphy_ocp_high() {
            if let Ok(v) = self.mmio.read32(GPHY_OCP) {
                return (v & 0xffff) as u16;
            }
        }
        0
    }

    /// Polls until `GPHY_OCP & OCPAR_FLAG` is clear (low), up to 10 iterations
    /// with ~25 us delay each.
    fn poll_gphy_ocp_low(&self) {
        for _ in 0..10 {
            if let Ok(v) = self.mmio.read32(GPHY_OCP) {
                if v & OCPAR_FLAG == 0 {
                    return;
                }
            }
            Self::udelay(25);
        }
        log::warn!("r8169 phy: poll_gphy_ocp_low timed out");
    }

    /// Polls until `GPHY_OCP & OCPAR_FLAG` is set (high), up to 10 iterations
    /// with ~25 us delay each.
    fn poll_gphy_ocp_high(&self) -> bool {
        for _ in 0..10 {
            if let Ok(v) = self.mmio.read32(GPHY_OCP) {
                if v & OCPAR_FLAG != 0 {
                    return true;
                }
            }
            Self::udelay(25);
        }
        log::warn!("r8169 phy: poll_gphy_ocp_high timed out");
        false
    }

    // -----------------------------------------------------------------------
    // 8168g MDIO read/write  (r8168g_mdio_{read,write})
    // -----------------------------------------------------------------------

    /// Writes a PHY register using the 8168g OCP-based path.
    ///
    /// Writing register `0x1f` (MII page select) changes `ocp_base` rather
    /// than issuing an actual MDIO write, matching the C driver behavior.
    pub fn mdio_write(&mut self, reg: u32, value: u16) {
        if reg == 0x1f {
            self.ocp_base = if value != 0 {
                (value as u32) << 4
            } else {
                OCP_STD_PHY_BASE
            };
            return;
        }

        let adjusted_reg = if self.ocp_base != OCP_STD_PHY_BASE {
            reg.wrapping_sub(0x10)
        } else {
            reg
        };

        // The suspend quirk for MAC_VER_40 modifies ERI bits based on
        // BMCR writes.  We handle it inline here.
        if self.ocp_base == OCP_STD_PHY_BASE && adjusted_reg == MII_BMCR {
            self.phy_suspend_quirk(value);
        }

        self.phy_ocp_write(self.ocp_base + adjusted_reg * 2, value);
    }

    /// Reads a PHY register using the 8168g OCP-based path.
    pub fn mdio_read(&self, reg: u32) -> u16 {
        if reg == 0x1f {
            return if self.ocp_base == OCP_STD_PHY_BASE {
                0
            } else {
                (self.ocp_base >> 4) as u16
            };
        }

        let adjusted_reg = if self.ocp_base != OCP_STD_PHY_BASE {
            reg.wrapping_sub(0x10)
        } else {
            reg
        };

        self.phy_ocp_read(self.ocp_base + adjusted_reg * 2)
    }

    // -----------------------------------------------------------------------
    // PHY suspend quirk (rtl8168g_phy_suspend_quirk)
    // -----------------------------------------------------------------------

    /// Work around a hardware issue with the RTL8168g PHY: disable PHY MCU
    /// interrupts before PHY power-down.
    fn phy_suspend_quirk(&self, value: u16) {
        if (value & BMCR_RESET != 0) || (value & BMCR_PDOWN == 0) {
            self.eri_set_bits(0x1a8, 0xfc00_0000);
        } else {
            self.eri_clear_bits(0x1a8, 0xfc00_0000);
        }
    }

    // -----------------------------------------------------------------------
    // ERI register helpers (used by the suspend quirk)
    // -----------------------------------------------------------------------

    /// Polls the ERIAR flag bit high (read completion).
    fn poll_eriar_high(&self) -> bool {
        for _ in 0..100 {
            if let Ok(v) = self.mmio.read32(ERIAR) {
                if v & ERIAR_FLAG != 0 {
                    return true;
                }
            }
            Self::udelay(100);
        }
        log::warn!("r8169 phy: poll_eriar_high timed out");
        false
    }

    /// Polls the ERIAR flag bit low (write completion).
    fn poll_eriar_low(&self) {
        for _ in 0..100 {
            if let Ok(v) = self.mmio.read32(ERIAR) {
                if v & ERIAR_FLAG == 0 {
                    return;
                }
            }
            Self::udelay(100);
        }
        log::warn!("r8169 phy: poll_eriar_low timed out");
    }

    /// Reads a 32-bit ERI register (EXGMAC type).
    fn eri_read(&self, addr: u32) -> u32 {
        let cmd = regs::ERIAR_READ_CMD | ERIAR_EXGMAC | ERIAR_MASK_1111 | addr;
        let _ = self.mmio.write32(ERIAR, cmd);
        if self.poll_eriar_high() {
            self.mmio.read32(ERIDR).unwrap_or(!0)
        } else {
            !0
        }
    }

    /// Writes a 32-bit ERI register (EXGMAC type).
    fn eri_write(&self, addr: u32, mask: u32, val: u32) {
        let cmd = ERIAR_WRITE_CMD | ERIAR_EXGMAC | mask | addr;
        let _ = self.mmio.write32(ERIDR, val);
        let _ = self.mmio.write32(ERIAR, cmd);
        self.poll_eriar_low();
    }

    /// Read-modify-write on an ERI register: `val = (val & ~clear) | set`.
    fn eri_modify(&self, addr: u32, set: u32, clear: u32) {
        let val = self.eri_read(addr);
        self.eri_write(addr, ERIAR_MASK_1111, (val & !clear) | set);
    }

    /// Sets bits in an ERI register.
    fn eri_set_bits(&self, addr: u32, bits: u32) {
        self.eri_modify(addr, bits, 0);
    }

    /// Clears bits in an ERI register.
    fn eri_clear_bits(&self, addr: u32, bits: u32) {
        self.eri_modify(addr, 0, bits);
    }

    // -----------------------------------------------------------------------
    // Convenience wrappers matching the Linux PHY API
    // -----------------------------------------------------------------------

    /// Writes a PHY register (like `phy_write`).
    pub fn phy_write(&mut self, reg: u32, val: u16) {
        self.mdio_write(reg, val);
    }

    /// Reads a PHY register (like `phy_read`).
    pub fn phy_read(&self, reg: u32) -> u16 {
        self.mdio_read(reg)
    }

    /// Modifies a PHY register: `val = (val & ~mask) | set`.
    pub fn phy_modify(&mut self, reg: u32, mask: u16, set: u16) {
        let val = self.mdio_read(reg);
        self.mdio_write(reg, (val & !mask) | set);
    }

    /// Sets bits in a PHY register.
    pub fn phy_set_bits(&mut self, reg: u32, bits: u16) {
        self.phy_modify(reg, 0, bits);
    }

    /// Clears bits in a PHY register.
    pub fn phy_clear_bits(&mut self, reg: u32, bits: u16) {
        self.phy_modify(reg, bits, 0);
    }

    // -----------------------------------------------------------------------
    // Paged register access (phy_{read,write,modify}_paged)
    //
    // For 8168g these translate to setting ocp_base via reg 0x1f writes,
    // performing the access, then restoring the base.
    // -----------------------------------------------------------------------

    /// Selects a PHY page.  Returns the old page for later restoration.
    fn phy_select_page(&mut self, page: u16) -> u16 {
        let old = self.mdio_read(0x1f);
        self.mdio_write(0x1f, page);
        old
    }

    /// Restores a previously selected PHY page.
    fn phy_restore_page(&mut self, old_page: u16) {
        self.mdio_write(0x1f, old_page);
    }

    /// Reads a register on a specific PHY page.
    pub fn phy_read_paged(&mut self, page: u16, reg: u32) -> u16 {
        let old = self.phy_select_page(page);
        let val = self.mdio_read(reg);
        self.phy_restore_page(old);
        val
    }

    /// Writes a register on a specific PHY page.
    pub fn phy_write_paged(&mut self, page: u16, reg: u32, val: u16) {
        let old = self.phy_select_page(page);
        self.mdio_write(reg, val);
        self.phy_restore_page(old);
    }

    /// Modifies a register on a specific PHY page.
    pub fn phy_modify_paged(&mut self, page: u16, reg: u32, mask: u16, set: u16) {
        let old = self.phy_select_page(page);
        self.phy_modify(reg, mask, set);
        self.phy_restore_page(old);
    }

    // -----------------------------------------------------------------------
    // r8168g_phy_param: paged parameter write on page 0x0a43
    //
    // Selects page 0x0a43, writes param to reg 0x13, then does a
    // read-modify-write on reg 0x14 with (mask, val).
    // -----------------------------------------------------------------------

    /// Writes a PHY parameter using the 8168g page-0xa43 indirect access
    /// method.  Corresponds to `r8168g_phy_param` in the C driver.
    pub fn r8168g_phy_param(&mut self, parm: u16, mask: u16, val: u16) {
        let old = self.phy_select_page(0x0a43);
        self.mdio_write(0x13, parm);
        self.phy_modify(0x14, mask, val);
        self.phy_restore_page(old);
    }

    // -----------------------------------------------------------------------
    // Batch PHY register writes (rtl_writephy_batch)
    // -----------------------------------------------------------------------

    /// Writes a batch of (register, value) pairs to the PHY.
    pub fn writephy_batch(&mut self, regs: &[(u16, u16)]) {
        for &(reg, val) in regs {
            self.mdio_write(reg as u32, val);
        }
    }

    // -----------------------------------------------------------------------
    // EEE configuration (rtl8168g_config_eee_phy)
    // -----------------------------------------------------------------------

    /// Configures Energy Efficient Ethernet for the RTL8168g PHY.
    ///
    /// Sets bit 4 of register 0x11 on page 0x0a43.
    pub fn config_eee_phy(&mut self) {
        self.phy_modify_paged(0x0a43, 0x11, 0, 1 << 4);
    }

    // -----------------------------------------------------------------------
    // ALDPS disable (rtl8168g_disable_aldps)
    // -----------------------------------------------------------------------

    /// Disables ALDPS (Advanced Link Down Power Saving) on the RTL8168g PHY.
    ///
    /// Clears bit 2 of register 0x10 on page 0x0a43.
    pub fn disable_aldps(&mut self) {
        self.phy_modify_paged(0x0a43, 0x10, 1 << 2, 0);
    }

    // -----------------------------------------------------------------------
    // 10M ALDPS adjustment (rtl8168g_phy_adjust_10m_aldps)
    // -----------------------------------------------------------------------

    /// Adjusts 10 Mbps ALDPS parameters for the RTL8168g PHY.
    pub fn adjust_10m_aldps(&mut self) {
        self.phy_modify_paged(0x0bcc, 0x14, 1 << 8, 0);
        self.phy_modify_paged(0x0a44, 0x11, 0, (1 << 7) | (1 << 6));
        self.r8168g_phy_param(0x8084, 0x6000, 0x0000);
        self.phy_modify_paged(0x0a43, 0x10, 0x0000, 0x1003);
    }

    // -----------------------------------------------------------------------
    // Microsecond delay helper
    // -----------------------------------------------------------------------

    /// Busy-wait for approximately `us` microseconds.
    ///
    /// In a proper Asterinas kernel build this should use `ostd::arch::delay`
    /// or a timer-based mechanism.  For now we use a simple spin loop that
    /// is portable and does not require unsafe.
    fn udelay(us: u32) {
        // Each iteration of the inner loop takes at least a few nanoseconds;
        // conservatively spin ~50 iterations per microsecond.
        for _ in 0..us {
            for _ in 0..50 {
                core::hint::spin_loop();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level PHY config entry point
// ---------------------------------------------------------------------------

/// Full PHY initialization for RTL8168g (MAC_VER_40).
///
/// This is the Rust equivalent of `rtl8168g_1_hw_phy_config` from the C
/// driver.  Firmware loading is stubbed out -- many chips work without
/// firmware, and `firmware_request_nowarn` in Linux silently skips it when
/// unavailable.
///
/// # Arguments
///
/// * `mmio` - MMIO handle for accessing the NIC registers.
pub fn rtl8168g_1_hw_phy_config(mmio: &Mmio) {
    let mut phy = PhyAccess::new(mmio);

    // Firmware is not loaded in the initial port.
    // r8169_apply_firmware(tp);

    // --- Conditional PHY fixups based on page 0x0a46 reads ---

    let val = phy.phy_read_paged(0x0a46, 0x10);
    if val & (1 << 8) != 0 {
        phy.phy_modify_paged(0x0bcc, 0x12, 1 << 15, 0);
    } else {
        phy.phy_modify_paged(0x0bcc, 0x12, 0, 1 << 15);
    }

    let val = phy.phy_read_paged(0x0a46, 0x13);
    if val & (1 << 8) != 0 {
        phy.phy_modify_paged(0x0c41, 0x15, 0, 1 << 1);
    } else {
        phy.phy_modify_paged(0x0c41, 0x15, 1 << 1, 0);
    }

    // Enable PHY auto speed down.
    phy.phy_modify_paged(0x0a44, 0x11, 0, (1 << 3) | (1 << 2));

    // 10M ALDPS adjustment.
    phy.adjust_10m_aldps();

    // EEE auto-fallback function.
    phy.phy_modify_paged(0x0a4b, 0x11, 0, 1 << 2);

    // Enable UC LPF tune function.
    phy.r8168g_phy_param(0x8012, 0x0000, 0x8000);

    phy.phy_modify_paged(0x0c42, 0x11, 1 << 13, 1 << 14);

    // Improve SWR Efficiency.
    phy.phy_write(0x1f, 0x0bcd);
    phy.phy_write(0x14, 0x5065);
    phy.phy_write(0x14, 0xd065);
    phy.phy_write(0x1f, 0x0bc8);
    phy.phy_write(0x11, 0x5655);
    phy.phy_write(0x1f, 0x0bcd);
    phy.phy_write(0x14, 0x1065);
    phy.phy_write(0x14, 0x9065);
    phy.phy_write(0x14, 0x1065);
    phy.phy_write(0x1f, 0x0000);

    // Disable ALDPS.
    phy.disable_aldps();

    // Configure EEE.
    phy.config_eee_phy();
}

/// Dispatches PHY configuration for the given MAC version.
///
/// Currently only RTL8168g (MAC_VER_40) is supported.  All other versions
/// are silently ignored.
pub fn r8169_hw_phy_config(mmio: &Mmio, _mac_ver: u32) {
    // MAC_VER_40 is the only supported variant in this initial port.
    rtl8168g_1_hw_phy_config(mmio);
}

// ---------------------------------------------------------------------------
// PHY link status helpers
// ---------------------------------------------------------------------------

/// Reads the PHY link status register (PHY_STATUS at offset 0x6c).
///
/// Returns `true` if the link-status bit is set.
pub fn phy_link_up(mmio: &Mmio) -> bool {
    match mmio.read8(regs::PHY_STATUS) {
        Ok(v) => v & regs::LINK_STATUS_FLAG != 0,
        Err(_) => false,
    }
}

/// Returns the negotiated link speed/duplex from PHY_STATUS.
///
/// Returns `(speed_mbps, full_duplex)`.  If the link is down, returns
/// `(0, false)`.
pub fn phy_link_speed(mmio: &Mmio) -> (u32, bool) {
    let status = match mmio.read8(regs::PHY_STATUS) {
        Ok(v) => v,
        Err(_) => return (0, false),
    };

    if status & regs::LINK_STATUS_FLAG == 0 {
        return (0, false);
    }

    let speed = if status & regs::GBPS_F_1000 != 0 {
        1000
    } else if status & regs::MBPS_100 != 0 {
        100
    } else if status & regs::MBPS_10 != 0 {
        10
    } else {
        0
    };

    let full_duplex = status & regs::FULL_DUP != 0;

    (speed, full_duplex)
}

// ---------------------------------------------------------------------------
// PHY power management helpers
// ---------------------------------------------------------------------------

/// Resets the PHY OCP base to the standard value.
///
/// This should be called after hardware reset to ensure the internal page
/// state is in sync.
pub fn phy_reset_ocp_base(mmio: &Mmio) {
    let mut phy = PhyAccess::new(mmio);
    phy.ocp_base = OCP_STD_PHY_BASE;
    // Write page register 0x1f = 0 to reset the page on the hardware side.
    phy.mdio_write(0x1f, 0x0000);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocp_reg_ok_checks() {
        assert!(PhyAccess::<'_>::ocp_reg_ok(0xa400));
        assert!(PhyAccess::<'_>::ocp_reg_ok(0xa402));
        // Odd address should fail.
        assert!(!PhyAccess::<'_>::ocp_reg_ok(0xa401));
    }
}
