// SPDX-License-Identifier: MPL-2.0

use alloc::borrow::ToOwned;
use core::slice::Iter;

use aster_bigtcp::{
    device::WithDevice,
    iface::{InterfaceFlags, InterfaceType},
};
use aster_softirq::BottomHalfDisabled;
use spin::Once;

use super::{Iface, poll::poll_ifaces};
use crate::{
    net::iface::{broadcast, sched::PollScheduler},
    prelude::*,
};

static IFACES: Once<Vec<Arc<Iface>>> = Once::new();

pub fn loopback_iface() -> &'static Arc<Iface> {
    &IFACES.get().unwrap()[0]
}

pub fn virtio_iface() -> Option<&'static Arc<Iface>> {
    IFACES.get().unwrap().get(1)
}

/// Returns the r8169 network interface, if present.
pub fn r8169_iface() -> Option<&'static Arc<Iface>> {
    IFACES
        .get()
        .unwrap()
        .iter()
        .find(|iface| iface.name() == R8169_IFACE_NAME)
}

/// Returns the e1000 network interface, if present.
pub fn e1000_iface() -> Option<&'static Arc<Iface>> {
    IFACES
        .get()
        .unwrap()
        .iter()
        .find(|iface| iface.name() == E1000_IFACE_NAME)
}

/// Returns the e1000e network interface, if present.
pub fn e1000e_iface() -> Option<&'static Arc<Iface>> {
    IFACES
        .get()
        .unwrap()
        .iter()
        .find(|iface| iface.name() == E1000E_IFACE_NAME)
}

/// Returns the first available Ethernet interface (virtio, r8169, e1000, or e1000e).
pub fn eth_iface() -> Option<&'static Arc<Iface>> {
    virtio_iface()
        .or_else(r8169_iface)
        .or_else(e1000_iface)
        .or_else(e1000e_iface)
}

pub fn iter_all_ifaces() -> Iter<'static, Arc<Iface>> {
    IFACES.get().unwrap().iter()
}

// TODO: Support multiple network devices and avoid the hardcoded device name.
const VIRTIO_DEVICE_NAME: &str = aster_virtio::device::network::DEVICE_NAME;
const R8169_DEVICE_NAME: &str = "r8169-net";
const R8169_IFACE_NAME: &str = "r8169";
const E1000_DEVICE_NAME: &str = aster_e1000::driver::DEVICE_NAME;
const E1000_IFACE_NAME: &str = "e1000";
const E1000E_DEVICE_NAME: &str = aster_e1000e::driver::DEVICE_NAME;
const E1000E_IFACE_NAME: &str = "e1000e";

pub fn init() {
    IFACES.call_once(|| {
        let mut ifaces = Vec::with_capacity(4);

        // Initialize loopback before other interfaces
        // to ensure the loopback interface index is ahead.
        ifaces.push(new_loopback());

        if let Some(iface_virtio) = new_virtio() {
            ifaces.push(iface_virtio);
        }

        if let Some(iface_r8169) = new_r8169() {
            ifaces.push(iface_r8169);
        }

        if let Some(iface_e1000) = new_e1000() {
            ifaces.push(iface_e1000);
        }

        if let Some(iface_e1000e) = new_e1000e() {
            ifaces.push(iface_e1000e);
        }

        ifaces
    });

    if let Some(iface_virtio) = virtio_iface() {
        let callback = || iface_virtio.poll();
        aster_network::register_recv_callback(VIRTIO_DEVICE_NAME, callback);
        aster_network::register_send_callback(VIRTIO_DEVICE_NAME, callback);
    }

    if let Some(iface_r8169) = r8169_iface() {
        let callback = || iface_r8169.poll();
        aster_network::register_recv_callback(R8169_DEVICE_NAME, callback);
        aster_network::register_send_callback(R8169_DEVICE_NAME, callback);
    }

    if let Some(iface_e1000) = e1000_iface() {
        let callback = || iface_e1000.poll();
        aster_network::register_recv_callback(E1000_DEVICE_NAME, callback);
        aster_network::register_send_callback(E1000_DEVICE_NAME, callback);
    }

    if let Some(iface_e1000e) = e1000e_iface() {
        let callback = || iface_e1000e.poll();
        aster_network::register_recv_callback(E1000E_DEVICE_NAME, callback);
        aster_network::register_send_callback(E1000E_DEVICE_NAME, callback);
    }

    broadcast::init();

    poll_ifaces();
}

fn new_loopback() -> Arc<Iface> {
    use aster_bigtcp::{
        device::{Loopback, Medium},
        iface::IpIface,
        wire::{Ipv4Address, Ipv4Cidr, Ipv6Address, Ipv6Cidr},
    };

    const LOOPBACK_ADDRESS: Ipv4Address = Ipv4Address::new(127, 0, 0, 1);
    const LOOPBACK_ADDRESS_PREFIX_LEN: u8 = 8; // mask: 255.0.0.0
    const LOOPBACK_IPV6_ADDRESS: Ipv6Address = Ipv6Address::new(0, 0, 0, 0, 0, 0, 0, 1);
    const LOOPBACK_IPV6_PREFIX_LEN: u8 = 128;

    struct Wrapper(Mutex<Loopback>);

    impl WithDevice for Wrapper {
        type Device = Loopback;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut Self::Device) -> R,
        {
            let mut device = self.0.lock();
            f(&mut device)
        }
    }

    // FIXME: These flags are currently hardcoded.
    // In the future, we should set appropriate values.
    let flags = InterfaceFlags::UP
        | InterfaceFlags::LOOPBACK
        | InterfaceFlags::RUNNING
        | InterfaceFlags::LOWER_UP;

    IpIface::new(
        Wrapper(Mutex::new(Loopback::new(Medium::Ip))),
        Ipv4Cidr::new(LOOPBACK_ADDRESS, LOOPBACK_ADDRESS_PREFIX_LEN),
        Some(Ipv6Cidr::new(
            LOOPBACK_IPV6_ADDRESS,
            LOOPBACK_IPV6_PREFIX_LEN,
        )),
        "lo".to_owned(),
        PollScheduler::new(),
        InterfaceType::LOOPBACK,
        flags,
    ) as Arc<Iface>
}

fn new_virtio() -> Option<Arc<Iface>> {
    use aster_bigtcp::{
        iface::EtherIface,
        wire::{EthernetAddress, Ipv4Address, Ipv4Cidr},
    };
    use aster_network::AnyNetworkDevice;

    const VIRTIO_ADDRESS: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
    const VIRTIO_ADDRESS_PREFIX_LEN: u8 = 24; // mask: 255.255.255.0
    const VIRTIO_GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

    let virtio_net = aster_network::get_device(VIRTIO_DEVICE_NAME)?;

    let ether_addr = virtio_net.lock().mac_addr().0;

    struct Wrapper(Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>);

    impl WithDevice for Wrapper {
        type Device = dyn AnyNetworkDevice;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut Self::Device) -> R,
        {
            let mut device = self.0.lock();
            f(&mut *device)
        }
    }

    // FIXME: These flags are currently hardcoded.
    // In the future, we should set appropriate values.
    let flags = InterfaceFlags::UP
        | InterfaceFlags::BROADCAST
        | InterfaceFlags::RUNNING
        | InterfaceFlags::MULTICAST
        | InterfaceFlags::LOWER_UP;

    Some(EtherIface::new(
        Wrapper(virtio_net),
        EthernetAddress(ether_addr),
        Ipv4Cidr::new(VIRTIO_ADDRESS, VIRTIO_ADDRESS_PREFIX_LEN),
        VIRTIO_GATEWAY,
        "eth0".to_owned(),
        PollScheduler::new(),
        flags,
    ))
}

fn new_r8169() -> Option<Arc<Iface>> {
    use aster_bigtcp::{
        iface::EtherIface,
        wire::{EthernetAddress, Ipv4Address, Ipv4Cidr},
    };
    use aster_network::AnyNetworkDevice;

    // TODO: Obtain these from DHCP or a configuration source.
    const R8169_ADDRESS: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
    const R8169_ADDRESS_PREFIX_LEN: u8 = 24; // mask: 255.255.255.0
    const R8169_GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

    let r8169_net = aster_network::get_device(R8169_DEVICE_NAME)?;

    let ether_addr = r8169_net.lock().mac_addr().0;

    struct Wrapper(Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>);

    impl WithDevice for Wrapper {
        type Device = dyn AnyNetworkDevice;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut Self::Device) -> R,
        {
            let mut device = self.0.lock();
            f(&mut *device)
        }
    }

    // FIXME: These flags are currently hardcoded.
    // In the future, we should set appropriate values.
    let flags = InterfaceFlags::UP
        | InterfaceFlags::BROADCAST
        | InterfaceFlags::RUNNING
        | InterfaceFlags::MULTICAST
        | InterfaceFlags::LOWER_UP;

    Some(EtherIface::new(
        Wrapper(r8169_net),
        EthernetAddress(ether_addr),
        Ipv4Cidr::new(R8169_ADDRESS, R8169_ADDRESS_PREFIX_LEN),
        R8169_GATEWAY,
        R8169_IFACE_NAME.to_owned(),
        PollScheduler::new(),
        flags,
    ))
}

fn new_e1000() -> Option<Arc<Iface>> {
    use aster_bigtcp::{
        iface::EtherIface,
        wire::{EthernetAddress, Ipv4Address, Ipv4Cidr},
    };
    use aster_network::AnyNetworkDevice;

    // TODO: Obtain these from DHCP or a configuration source.
    const E1000_ADDRESS: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
    const E1000_ADDRESS_PREFIX_LEN: u8 = 24; // mask: 255.255.255.0
    const E1000_GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

    let e1000_net = aster_network::get_device(E1000_DEVICE_NAME)?;

    let ether_addr = e1000_net.lock().mac_addr().0;

    struct Wrapper(Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>);

    impl WithDevice for Wrapper {
        type Device = dyn AnyNetworkDevice;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut Self::Device) -> R,
        {
            let mut device = self.0.lock();
            f(&mut *device)
        }
    }

    // FIXME: These flags are currently hardcoded.
    // In the future, we should set appropriate values.
    let flags = InterfaceFlags::UP
        | InterfaceFlags::BROADCAST
        | InterfaceFlags::RUNNING
        | InterfaceFlags::MULTICAST
        | InterfaceFlags::LOWER_UP;

    Some(EtherIface::new(
        Wrapper(e1000_net),
        EthernetAddress(ether_addr),
        Ipv4Cidr::new(E1000_ADDRESS, E1000_ADDRESS_PREFIX_LEN),
        E1000_GATEWAY,
        E1000_IFACE_NAME.to_owned(),
        PollScheduler::new(),
        flags,
    ))
}

fn new_e1000e() -> Option<Arc<Iface>> {
    use aster_bigtcp::{
        iface::EtherIface,
        wire::{EthernetAddress, Ipv4Address, Ipv4Cidr},
    };
    use aster_network::AnyNetworkDevice;

    // TODO: Obtain these from DHCP or a configuration source.
    const E1000E_ADDRESS: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
    const E1000E_ADDRESS_PREFIX_LEN: u8 = 24; // mask: 255.255.255.0
    const E1000E_GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

    let e1000e_net = aster_network::get_device(E1000E_DEVICE_NAME)?;

    let ether_addr = e1000e_net.lock().mac_addr().0;

    struct Wrapper(Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>);

    impl WithDevice for Wrapper {
        type Device = dyn AnyNetworkDevice;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut Self::Device) -> R,
        {
            let mut device = self.0.lock();
            f(&mut *device)
        }
    }

    // FIXME: These flags are currently hardcoded.
    // In the future, we should set appropriate values.
    let flags = InterfaceFlags::UP
        | InterfaceFlags::BROADCAST
        | InterfaceFlags::RUNNING
        | InterfaceFlags::MULTICAST
        | InterfaceFlags::LOWER_UP;

    Some(EtherIface::new(
        Wrapper(e1000e_net),
        EthernetAddress(ether_addr),
        Ipv4Cidr::new(E1000E_ADDRESS, E1000E_ADDRESS_PREFIX_LEN),
        E1000E_GATEWAY,
        E1000E_IFACE_NAME.to_owned(),
        PollScheduler::new(),
        flags,
    ))
}
