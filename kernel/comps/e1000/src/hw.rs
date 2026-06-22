// SPDX-License-Identifier: MPL-2.0

//! Hardware abstraction for the e1000 82540EM.
//! Provides MAC type detection, hardware reset, initialization, link setup,
//! flow control, MAC address read, adaptive IFS, and collision distance config.
//! Translated from e1000_hw.c focusing on 82540EM paths.

use crate::regs::*;

/// Flow control configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowControlMode {
    None,
    RxPause,
    TxPause,
    Full,
}

/// Link speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSpeed {
    Speed10,
    Speed100,
    Speed1000,
}

/// Duplex mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Duplex {
    Half,
    Full,
}

/// Hardware state for the e1000 NIC (fields relevant to 82540EM).
pub struct E1000Hw {
    /// MMIO register accessor.
    pub regs: E1000Regs,
    /// Current flow control mode.
    pub fc: FlowControlMode,
    /// Original FC setting (for reset recovery).
    pub original_fc: FlowControlMode,
    /// FC autoneg enabled.
    pub fc_autoneg: bool,
    /// FC high watermark.
    pub fc_high_water: u16,
    /// FC low watermark.
    pub fc_low_water: u16,
    /// FC pause time.
    pub fc_pause_time: u16,
    /// FC send XON.
    pub fc_send_xon: bool,
    /// MAC address (permanent).
    pub perm_mac_addr: [u8; 6],
    /// MAC address (active).
    pub mac_addr: [u8; 6],
    /// PHY address on MDIO bus (typically 1 for 82540EM).
    pub phy_addr: u32,
    /// PHY ID read from hardware.
    pub phy_id: u32,
    /// Autoneg advertised capabilities.
    pub autoneg_advertised: u16,
    /// Whether autoneg is enabled.
    pub autoneg: bool,
    /// Forced speed/duplex setting.
    pub forced_speed_duplex: u8,
    /// Whether to wait for autoneg completion.
    pub wait_autoneg_complete: bool,
    /// Link status tracking.
    pub get_link_status: bool,
    /// Adaptive IFS enabled.
    pub adaptive_ifs: bool,
    pub ifs_params_forced: bool,
    pub in_ifs_mode: bool,
    pub current_ifs_val: u16,
    pub ifs_min_val: u16,
    pub ifs_max_val: u16,
    pub ifs_step_size: u16,
    pub ifs_ratio: u16,
    /// Collision delta (for adaptive IFS).
    pub collision_delta: u32,
    /// TX packet delta (for adaptive IFS).
    pub tx_packet_delta: u32,
}

impl E1000Hw {
    /// Creates a new hardware abstraction from the given MMIO register accessor.
    pub fn new(regs: E1000Regs) -> Self {
        Self {
            regs,
            fc: FlowControlMode::Full,
            original_fc: FlowControlMode::Full,
            fc_autoneg: true,
            fc_high_water: 0,
            fc_low_water: 0,
            fc_pause_time: 0xFFFF,
            fc_send_xon: true,
            perm_mac_addr: [0; 6],
            mac_addr: [0; 6],
            phy_addr: 1,
            phy_id: 0,
            autoneg_advertised: 0x2F, // 10/100/1000 full/half
            autoneg: true,
            forced_speed_duplex: 0,
            wait_autoneg_complete: false,
            get_link_status: true,
            adaptive_ifs: true,
            ifs_params_forced: false,
            in_ifs_mode: false,
            current_ifs_val: 0,
            ifs_min_val: 0,
            ifs_max_val: 0,
            ifs_step_size: 0,
            ifs_ratio: 0,
            collision_delta: 0,
            tx_packet_delta: 0,
        }
    }

    // ========================================================================
    // Hardware Reset
    // ========================================================================

    /// Performs a full hardware reset of the 82540EM.
    pub fn reset_hw(&self) {
        // Mask off all interrupts
        self.regs.write(IMC, 0xFFFF_FFFF);

        // Disable receive and transmit
        self.regs.write(RCTL, 0);
        self.regs.write(TCTL, TCTL_PSP);

        // Flush pending DMA by reading STATUS
        let _ = self.regs.read(STATUS);

        // Delay to allow outstanding PCI transactions to complete
        self.delay_us(10);

        // Issue global reset
        let ctrl = self.regs.read(CTRL);
        self.regs.write(CTRL, ctrl | CTRL_RST);

        // Wait for reset to complete (at least 1ms per datasheet)
        self.delay_us(2000);

        // After reset, clear interrupt masks again
        self.regs.write(IMC, 0xFFFF_FFFF);
        // Clear any pending interrupts
        let _ = self.regs.read(ICR);
    }

    // ========================================================================
    // Hardware Initialization
    // ========================================================================

    /// Initializes the hardware after reset.
    /// Sets up the receive address, multicast table, VLAN filter,
    /// and flow control registers.
    pub fn init_hw(&mut self) -> Result<(), &'static str> {
        // Initialize identification LED
        let ledctl = self.regs.read(LEDCTL);
        self.regs.write(LEDCTL, ledctl);

        // Set up the receive address (RAR 0 = own MAC)
        self.rar_set(&self.mac_addr.clone(), 0);

        // Zero out the Multicast Table Array
        for i in 0..NUM_MTA_REGISTERS {
            self.regs.write(MTA + (i * 4), 0);
        }

        // Setup link and flow control
        self.setup_link()?;

        // Clear all statistics registers by reading them
        self.clear_hw_counters();

        Ok(())
    }

    // ========================================================================
    // Link Setup
    // ========================================================================

    /// Sets up link speed, duplex, and flow control.
    pub fn setup_link(&mut self) -> Result<(), &'static str> {
        // Read MAC address from EEPROM if not already set
        if self.perm_mac_addr == [0u8; 6] {
            self.read_mac_addr()?;
        }

        // Set flow control registers (for 82540EM copper, use CTRL bits)
        self.fc = self.original_fc;

        // Setup the PHY and copper link
        self.setup_copper_link()?;

        // Configure flow control after link is established
        self.config_fc_after_link_up()?;

        Ok(())
    }

    /// Sets up the copper PHY link (82540EM uses M88 PHY).
    fn setup_copper_link(&mut self) -> Result<(), &'static str> {
        // Setup PHY autoneg or forced speed/duplex via the PHY module
        crate::phy::setup_copper_link(self)?;
        Ok(())
    }

    /// Configures flow control parameters after link is up.
    fn config_fc_after_link_up(&self) -> Result<(), &'static str> {
        self.force_mac_fc()?;
        Ok(())
    }

    /// Forces the MAC flow control settings into the CTRL register.
    pub fn force_mac_fc(&self) -> Result<(), &'static str> {
        let mut ctrl = self.regs.read(CTRL);

        // Clear both FC bits first
        ctrl &= !(CTRL_TFCE | CTRL_RFCE);

        match self.fc {
            FlowControlMode::None => {}
            FlowControlMode::RxPause => {
                ctrl |= CTRL_RFCE;
            }
            FlowControlMode::TxPause => {
                ctrl |= CTRL_TFCE;
            }
            FlowControlMode::Full => {
                ctrl |= CTRL_RFCE | CTRL_TFCE;
            }
        }

        self.regs.write(CTRL, ctrl);
        Ok(())
    }

    // ========================================================================
    // Link Status
    // ========================================================================

    /// Checks for link and returns (link_up, speed, duplex).
    pub fn check_for_link(&mut self) -> (bool, Option<LinkSpeed>, Option<Duplex>) {
        let status = self.regs.read(STATUS);
        let link_up = (status & STATUS_LU) != 0;

        if !link_up {
            self.get_link_status = true;
            return (false, None, None);
        }

        self.get_link_status = false;

        let speed = match status & STATUS_SPEED_MASK {
            STATUS_SPEED_10 => LinkSpeed::Speed10,
            STATUS_SPEED_100 => LinkSpeed::Speed100,
            STATUS_SPEED_1000 => LinkSpeed::Speed1000,
            _ => LinkSpeed::Speed1000,
        };

        let duplex = if (status & STATUS_FD) != 0 {
            Duplex::Full
        } else {
            Duplex::Half
        };

        (true, Some(speed), Some(duplex))
    }

    /// Returns the current link speed and duplex by reading STATUS register.
    pub fn get_speed_and_duplex(&self) -> (u16, u16) {
        let status = self.regs.read(STATUS);
        let speed = match status & STATUS_SPEED_MASK {
            STATUS_SPEED_10 => SPEED_10,
            STATUS_SPEED_100 => SPEED_100,
            _ => SPEED_1000,
        };
        let duplex = if (status & STATUS_FD) != 0 {
            FULL_DUPLEX
        } else {
            HALF_DUPLEX
        };
        (speed, duplex)
    }

    // ========================================================================
    // MAC Address
    // ========================================================================

    /// Reads the MAC address from the EEPROM (via EERD register for 82540EM).
    pub fn read_mac_addr(&mut self) -> Result<(), &'static str> {
        let mut mac = [0u8; 6];
        for i in 0..3 {
            let word = crate::eeprom::read_eeprom(self, EEPROM_ENET_ADDR + i as u16)?;
            mac[i * 2] = (word & 0xFF) as u8;
            mac[i * 2 + 1] = (word >> 8) as u8;
        }
        self.perm_mac_addr = mac;
        self.mac_addr = mac;
        Ok(())
    }

    /// Programs a receive address into the specified RAR slot.
    pub fn rar_set(&self, addr: &[u8; 6], index: u32) {
        let ral = (addr[0] as u32)
            | ((addr[1] as u32) << 8)
            | ((addr[2] as u32) << 16)
            | ((addr[3] as u32) << 24);
        let rah = (addr[4] as u32) | ((addr[5] as u32) << 8) | RAH_AV;

        self.regs.write(RA + (index as usize * 8), ral);
        self.regs.write(RA + (index as usize * 8) + 4, rah);
    }

    // ========================================================================
    // Collision Distance
    // ========================================================================

    /// Configures the collision distance in the TCTL register.
    pub fn config_collision_dist(&self) {
        let mut tctl = self.regs.read(TCTL);
        tctl &= !E1000_TCTL_COLD;
        tctl |= COLLISION_DISTANCE_FD << TCTL_COLD_SHIFT;
        self.regs.write(TCTL, tctl);
        // Flush
        let _ = self.regs.read(STATUS);
    }

    // ========================================================================
    // Adaptive IFS
    // ========================================================================

    /// Resets adaptive IFS state.
    pub fn reset_adaptive(&mut self) {
        self.current_ifs_val = 0;
        self.ifs_min_val = IFS_MIN;
        self.ifs_max_val = IFS_MAX;
        self.ifs_step_size = IFS_STEP;
        self.ifs_ratio = IFS_RATIO;
        self.in_ifs_mode = false;
        self.regs.write(AIT, 0);
    }

    /// Updates adaptive IFS based on collision/packet ratio.
    pub fn update_adaptive(&mut self) {
        if !self.adaptive_ifs {
            return;
        }
        if self.ifs_params_forced {
            return;
        }

        if self.tx_packet_delta > MIN_NUM_XMITS
            && self.collision_delta * self.ifs_ratio as u32
                > self.tx_packet_delta * self.ifs_step_size as u32
        {
            if self.current_ifs_val < self.ifs_max_val {
                if self.current_ifs_val == 0 {
                    self.current_ifs_val = self.ifs_min_val;
                } else {
                    self.current_ifs_val += self.ifs_step_size;
                }
                self.regs.write(AIT, self.current_ifs_val as u32);
                self.in_ifs_mode = true;
            }
        } else if self.in_ifs_mode
            && self.tx_packet_delta <= MIN_NUM_XMITS
        {
            self.current_ifs_val = 0;
            self.in_ifs_mode = false;
            self.regs.write(AIT, 0);
        }
    }

    // ========================================================================
    // Multicast
    // ========================================================================

    /// Computes the hash value for a multicast address.
    pub fn hash_mc_addr(&self, mc_addr: &[u8; 6]) -> u32 {
        // For 82540EM, use bits [47:36] of the address
        let hash_value = ((mc_addr[4] as u32) >> 4) | ((mc_addr[5] as u32) << 4);
        hash_value & 0xFFF
    }

    /// Programs the multicast address list into the MTA.
    pub fn update_mc_addr_list(&self, mc_addrs: &[[u8; 6]]) {
        // Clear the MTA
        for i in 0..NUM_MTA_REGISTERS {
            self.regs.write(MTA + (i * 4), 0);
        }
        // Set bits for each multicast address
        for addr in mc_addrs {
            let hash = self.hash_mc_addr(addr);
            let reg_index = (hash >> 5) as usize;
            let bit_index = hash & 0x1F;
            let mta_val = self.regs.read(MTA + (reg_index * 4));
            self.regs.write(MTA + (reg_index * 4), mta_val | (1 << bit_index));
        }
    }

    // ========================================================================
    // VLAN Filter
    // ========================================================================

    /// Writes a value to the VLAN filter table at the given offset.
    pub fn write_vfta(&self, offset: u32, value: u32) {
        self.regs.write(VFTA + (offset as usize * 4), value);
    }

    // ========================================================================
    // Statistics
    // ========================================================================

    /// Clears hardware statistics counters by reading them.
    pub fn clear_hw_counters(&self) {
        let _ = self.regs.read(CRCERRS);
        let _ = self.regs.read(ALGNERRC);
        let _ = self.regs.read(SYMERRS);
        let _ = self.regs.read(RXERRC);
        let _ = self.regs.read(MPC);
        let _ = self.regs.read(SCC);
        let _ = self.regs.read(ECOL);
        let _ = self.regs.read(MCC);
        let _ = self.regs.read(LATECOL);
        let _ = self.regs.read(COLC);
        let _ = self.regs.read(DC);
        let _ = self.regs.read(TNCRS);
        let _ = self.regs.read(CEXTERR);
        let _ = self.regs.read(RLEC);
        let _ = self.regs.read(XONRXC);
        let _ = self.regs.read(XONTXC);
        let _ = self.regs.read(XOFFRXC);
        let _ = self.regs.read(XOFFTXC);
        let _ = self.regs.read(FCRUC);
        let _ = self.regs.read(GPRC);
        let _ = self.regs.read(BPRC);
        let _ = self.regs.read(MPRC);
        let _ = self.regs.read(GPTC);
        let _ = self.regs.read(GORCL);
        let _ = self.regs.read(GORCH);
        let _ = self.regs.read(GOTCL);
        let _ = self.regs.read(GOTCH);
        let _ = self.regs.read(RNBC);
        let _ = self.regs.read(RUC);
        let _ = self.regs.read(RFC);
        let _ = self.regs.read(ROC);
        let _ = self.regs.read(RJC);
        let _ = self.regs.read(TORL);
        let _ = self.regs.read(TORH);
        let _ = self.regs.read(TOTL);
        let _ = self.regs.read(TOTH);
        let _ = self.regs.read(TPR);
        let _ = self.regs.read(TPT);
        let _ = self.regs.read(MPTC);
        let _ = self.regs.read(BPTC);
        let _ = self.regs.read(TSCTC);
        let _ = self.regs.read(TSCTFC);
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    /// Microsecond delay (busy-loop approximation).
    fn delay_us(&self, us: u32) {
        // In kernel context we use a simple spin-loop.
        // Each iteration of a volatile read is ~1 cycle at ~1GHz = ~1ns.
        // We conservatively do 100 iters per us.
        for _ in 0..(us as u64 * 100) {
            core::hint::spin_loop();
        }
    }
}

// TCTL collision distance mask (for clearing)
const E1000_TCTL_COLD: u32 = 0x003FF000;

// Adaptive IFS constants
const AIT: usize = 0x00458;
const IFS_MIN: u16 = 64;
const IFS_MAX: u16 = 1000;
const IFS_STEP: u16 = 8;
const IFS_RATIO: u16 = 4;
const MIN_NUM_XMITS: u32 = 1000;
