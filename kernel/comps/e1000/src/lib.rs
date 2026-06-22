// SPDX-License-Identifier: MPL-2.0

//! Intel e1000 (82540EM) network driver for Asterinas OS.
//!
//! This crate provides a PCI network driver for the Intel 82540EM Gigabit
//! Ethernet controller, commonly emulated by QEMU/KVM and VirtualBox.

#![no_std]
#![feature(trait_alias)]

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "e1000: "
    };
}

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

mod desc;
pub mod driver;
mod eeprom;
pub mod hw;
mod intr;
mod phy;
pub mod regs;
mod rx;
mod tx;

use alloc::sync::Arc;

use component::{ComponentInitError, init_component};

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    // Register the e1000 PCI driver with the PCI bus.
    let pci_driver = Arc::new(driver::E1000PciDriver);
    aster_pci::PCI_BUS.lock().register_driver(pci_driver);
    Ok(())
}
