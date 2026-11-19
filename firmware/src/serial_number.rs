//! A number unique to the board: the unique ID of its flash chip.

use defmt::warn;
use embassy_rp::{
    Peri,
    flash::{self, Flash},
    peripherals::FLASH,
};

/// Pico and Pico W.
const FLASH_SIZE: usize = 2 * 1024 * 1024;

/// Reads the flash chip's unique ID, or 0 if that fails.
///
/// Flash access pauses core 1, so call this before starting it.
pub fn read(flash: Peri<'static, FLASH>) -> u64 {
    let mut flash = Flash::<_, flash::Blocking, FLASH_SIZE>::new_blocking(flash);
    let mut uid = [0; 8];
    if let Err(e) = flash.blocking_unique_id(&mut uid) {
        warn!("Failed to read flash unique id: {}", e);
    }
    u64::from_be_bytes(uid)
}
