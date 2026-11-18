use embassy_time::Duration;

pub const OUTGOING_CHANNEL_CAPACITY: usize = 16;
pub const INCOMING_CHANNEL_CAPACITY: usize = 8;

pub const CORE1_STACK_SIZE: usize = 16 * 1024;

/// Raspberry Pi Pico.
pub const FLASH_SIZE: usize = 2 * 1024 * 1024;

pub const SENSOR_INTERVAL: Duration = Duration::from_millis(500);

/// How long the host may take to collect each USB packet the device sends, see `rpc::configure_sender`.
/// While no host is reading, every sensor reading blocks the sender this long, so it must stay well below
/// `SENSOR_INTERVAL`.
pub const SEND_TIMEOUT_MS_PER_PACKET: usize = 50;
