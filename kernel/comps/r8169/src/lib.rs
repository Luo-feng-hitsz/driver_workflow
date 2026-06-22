// SPDX-License-Identifier: MPL-2.0

//! RTL8168g (r8169 family) network driver for Asterinas OS.
#![no_std]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::sync::Arc;

use aster_pci::PCI_BUS;
use component::{ComponentInitError, init_component};

use crate::device::R8169PciDriver;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "r8169: "
    };
}

pub mod regs;
pub mod desc;
pub mod phy;
pub mod tx;
pub mod rx;
pub mod device;

#[init_component]
fn r8169_component_init() -> Result<(), ComponentInitError> {
    PCI_BUS
        .lock()
        .register_driver(Arc::new(R8169PciDriver));
    Ok(())
}
