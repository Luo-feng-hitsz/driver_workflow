// SPDX-License-Identifier: MPL-2.0

//! PHY register access and link configuration for the Intel 82574L (e1000e).
//!
//! Handles MDIC/BM2 read/write, link setup, copper link configuration,
//! polarity check, cable length, PHY info, speed/duplex forcing, PHY reset,
//! and LPLU state management.
//!
//! Translated from: e1000_phy.c (generic + BM2/M88 paths used by 82574)

// TODO: Populate with PHY operations.
