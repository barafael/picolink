//! Running a second executor on core 1.
//!
//! Each core has its own executor, so tasks never move between cores. Only share data between them through
//! types that are safe across cores, for example embassy-sync channels with a `CriticalSectionRawMutex`.

use embassy_executor::{Executor, Spawner};
use embassy_rp::{
    Peri,
    multicore::{Stack, spawn_core1 as start},
    peripherals::CORE1,
};
use static_cell::StaticCell;

const CORE1_STACK_SIZE: usize = 16 * 1024;

static CORE1_STACK: StaticCell<Stack<CORE1_STACK_SIZE>> = StaticCell::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

/// Starts core 1 with an executor, and calls `init` on it to spawn the first tasks. Panics if called twice.
///
/// Peripherals that enable an interrupt, like the USB driver, enable it on the core that creates them.
/// So create those inside `init` to have their interrupts handled on core 1.
pub fn spawn_core1(core1: Peri<'static, CORE1>, init: impl FnOnce(Spawner) + Send + 'static) {
    start(core1, CORE1_STACK.init(Stack::new()), move || {
        EXECUTOR1.init(Executor::new()).run(init)
    });
}
