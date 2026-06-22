// SPDX-License-Identifier: MPL-2.0

//! DMA buffer pool initialization for the e1000 driver.

use alloc::sync::Arc;

use aster_network::dma_pool::DmaPool;
use ostd::mm::dma::{FromDevice, ToDevice};
use spin::Once;

use crate::regs::{RX_BUFFER_SIZE, TX_BUFFER_SIZE};

pub(crate) static RX_BUFFER_POOL: Once<Arc<DmaPool<FromDevice>>> = Once::new();
pub(crate) static TX_BUFFER_POOL: Once<Arc<DmaPool<ToDevice>>> = Once::new();

const POOL_INIT_SIZE: usize = 32;
const POOL_HIGH_WATERMARK: usize = 64;

pub(crate) fn init() {
    RX_BUFFER_POOL
        .call_once(|| DmaPool::new(RX_BUFFER_SIZE, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false));
    TX_BUFFER_POOL
        .call_once(|| DmaPool::new(TX_BUFFER_SIZE, POOL_INIT_SIZE, POOL_HIGH_WATERMARK, false));
}
