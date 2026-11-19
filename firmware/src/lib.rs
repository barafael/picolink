//! Building blocks for picolink firmwares on the RP2040.
//!
//! - [`link`]: the USB device and a postcard-rpc server for the firmware's own endpoints.
//! - [`serial_number`]: a number unique to the board, for telling devices apart.
//! - [`multicore`]: running a second executor on core 1.
//! - [`temperature`]: the RP2040's internal temperature sensor.
//!
//! The examples are complete firmwares built from these:
//! - `sensor_actuator`: a plain Pico serving its temperature sensor and onboard LED, on both cores.
//! - `pico_w_wifi`: a Pico W that scans for Wi-Fi networks and joins one, on one core.

#![no_std]
#![forbid(unsafe_code)]

pub mod link;
pub mod multicore;
pub mod serial_number;
pub mod temperature;
