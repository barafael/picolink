//! A device with one sensor and one actuator: on a plain Pico, the chip's temperature sensor and the onboard LED.

use postcard_rpc::{TopicDirection, endpoints, topics};
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

/// The server and the WebUSB app only claim devices with this USB product string.
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
