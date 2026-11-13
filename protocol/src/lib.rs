//! The interface between the picolink firmware and its host, for [postcard-rpc](https://docs.rs/postcard-rpc).
//!
//! Endpoints are request/response pairs, topics are one-way messages.
//! postcard-rpc identifies both by a hash of path and schema, so a host and a device built from different
//! versions of this crate reject each other's messages instead of misinterpreting them.

#![no_std]

use postcard_rpc::{TopicDirection, endpoints, topics};
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

/// Same VID/PID as the MT21: the host tells the two apart by [`USB_PRODUCT`].
pub const USB_VID: u16 = 0x16D0;
pub const USB_PID: u16 = 0x12CE;

pub const USB_MANUFACTURER: &str = "picolink";
pub const USB_PRODUCT: &str = "picolink";

endpoints! {
    list = ENDPOINT_LIST;
    | EndpointTy            | RequestTy     | ResponseTy    | Path              |
    | ----------            | ---------     | ----------    | ----              |
    | SerialNumberEndpoint  | ()            | u64           | "serial_number"   |
    | GetActuatorEndpoint   | ()            | ActuatorState | "actuator/get"    |
    | SetActuatorEndpoint   | ActuatorState | ActuatorState | "actuator/set"    |
}

topics! {
    list = TOPICS_IN_LIST;
    direction = TopicDirection::ToServer;
    | TopicTy               | MessageTy     | Path              |
    | -------               | ---------     | ----              |
}

topics! {
    list = TOPICS_OUT_LIST;
    direction = TopicDirection::ToClient;
    | TopicTy               | MessageTy     | Path              |
    | -------               | ---------     | ----              |
    | ReadingTopic          | SensorReading | "sensor/reading"  |
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ActuatorState {
    #[default]
    Off,
    On,
}

impl ActuatorState {
    #[must_use]
    pub fn toggled(self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Off,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Schema)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SensorReading {
    /// Device uptime at the time of the reading.
    pub uptime_ms: u64,
    pub celsius: f32,
}
