// SPDX-License-Identifier: MPL-2.0

//! PCI driver probe logic for the Intel e1000.
//!
//! Implements the `PciDriver` trait: checks vendor 0x8086, device 0x100E (and
//! related 82540EM variants), maps BAR0, and initializes the device.

use alloc::sync::Arc;
use core::fmt::Debug;

use aster_pci::{
    PciDeviceId,
    bus::{PciDevice, PciDriver},
    cfg_space::BarAccess,
    common_device::PciCommonDevice,
};
use ostd::bus::BusProbeError;

use crate::{device::E1000Device, regs};

#[derive(Debug)]
struct E1000PciDevice {
    device_id: PciDeviceId,
}

impl PciDevice for E1000PciDevice {
    fn device_id(&self) -> PciDeviceId {
        self.device_id
    }
}

#[derive(Debug)]
pub(crate) struct E1000PciDriver;

impl E1000PciDriver {
    pub(crate) fn new() -> Self {
        Self
    }
}

impl PciDriver for E1000PciDriver {
    fn probe(
        &self,
        mut device: PciCommonDevice,
    ) -> Result<Arc<dyn PciDevice>, (BusProbeError, PciCommonDevice)> {
        let dev_id = device.device_id();

        if dev_id.vendor_id != regs::INTEL_VENDOR_ID {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        if !regs::SUPPORTED_DEVICE_IDS.contains(&dev_id.device_id) {
            return Err((BusProbeError::DeviceNotMatch, device));
        }

        ostd::info!(
            "found Intel e1000 NIC: {:04x}:{:04x}",
            dev_id.vendor_id,
            dev_id.device_id
        );

        // Acquire BAR0 MMIO region
        let io_mem = {
            let bar0 = match device.bar_manager_mut().bar_mut(0) {
                Some(bar) => bar,
                None => return Err((BusProbeError::ConfigurationSpaceError, device)),
            };
            match bar0.acquire() {
                Ok(BarAccess::Memory(io_mem)) => io_mem,
                Ok(BarAccess::Io) => {
                    return Err((BusProbeError::ConfigurationSpaceError, device));
                }
                Err(_) => {
                    return Err((BusProbeError::ConfigurationSpaceError, device));
                }
            }
        };

        // MSI-X is optional; e1000 can fall back to legacy interrupts
        let msix = device.acquire_msix_capability().ok().flatten();

        let device_id = *device.device_id();
        let location = *device.location();

        E1000Device::init(io_mem, msix, location).map_err(|err| {
            ostd::error!("e1000: device initialization failed: {:?}", err);
            (BusProbeError::ConfigurationSpaceError, device)
        })?;

        Ok(Arc::new(E1000PciDevice { device_id }))
    }
}
