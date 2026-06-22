// SPDX-License-Identifier: MPL-2.0

//! Intel e1000 network driver for Asterinas.
//!
//! This crate implements the Intel 82540EM (e1000) network interface controller
//! driver as an Asterinas component. It supports basic packet send/receive over
//! PCI with MMIO register access.

#![no_std]
#![deny(unsafe_code)]

extern crate alloc;
#[macro_use]
extern crate ostd_pod;

// Set this crate's log prefix for `ostd::log`.
macro_rules! __log_prefix {
    () => {
        "e1000: "
    };
}

mod buffer;
mod desc;
mod device;
mod driver;
mod hw;
mod regs;

use alloc::sync::Arc;

use component::{ComponentInitError, init_component};

pub use self::device::DEVICE_NAME;
use self::driver::E1000PciDriver;

#[init_component]
fn e1000_component_init() -> Result<(), ComponentInitError> {
    buffer::init();
    let driver = Arc::new(E1000PciDriver::new());
    aster_pci::PCI_BUS.lock().register_driver(driver);
    Ok(())
}
