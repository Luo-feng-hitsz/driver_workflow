// SPDX-License-Identifier: MPL-2.0

//! Intel 82574L (e1000e) network driver for Asterinas OS.
//!
//! This crate provides a PCI network driver for the Intel 82574L Gigabit
//! Ethernet controller (e1000e family). The driver implements PCI probe via
//! the `aster-pci` bus, programs the NIC through MMIO registers, and exposes
//! the standard `AnyNetworkDevice` trait from `aster-network` for integration
//! with the Asterinas networking stack.

#![no_std]
#![deny(unsafe_code)]

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "e1000e: "
    };
}

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

mod desc;
pub mod driver;
mod mac;
mod nvm;
mod phy;
mod regs;
mod rx;
mod tx;

use alloc::sync::Arc;

use component::{ComponentInitError, init_component};

#[init_component]
fn init() -> Result<(), ComponentInitError> {
    // Register the e1000e PCI driver with the PCI bus.
    let pci_driver = Arc::new(driver::E1000ePciDriver);
    aster_pci::PCI_BUS.lock().register_driver(pci_driver);
    Ok(())
}
