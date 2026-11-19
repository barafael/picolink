//! The interfaces between picolink firmwares and their hosts, for [postcard-rpc](https://docs.rs/postcard-rpc).
//!
//! Endpoints are request/response pairs, topics are one-way messages.
//! postcard-rpc identifies both by a hash of path and schema, so a host and a device built from different
//! versions of this crate reject each other's messages instead of misinterpreting them.
//!
//! Every firmware serves [`SerialNumberEndpoint`], plus the endpoints and topics of its own module:
//! - [`sensor_actuator`]: a temperature sensor and an LED.
//! - [`wifi`]: scanning for and joining Wi-Fi networks, on a Pico W.
//!
//! Each firmware's USB product string is in its module. Hosts use it to find the device they understand.

#![cfg_attr(not(test), no_std)]

use postcard_rpc::endpoints;

pub mod sensor_actuator;
pub mod wifi;

/// Same VID/PID as the MT21: the host tells them apart by the USB product string.
pub const USB_VID: u16 = 0x16D0;
pub const USB_PID: u16 = 0x12CE;

pub const USB_MANUFACTURER: &str = "picolink";

endpoints! {
    list = COMMON_ENDPOINT_LIST;
    | EndpointTy            | RequestTy     | ResponseTy    | Path              |
    | ----------            | ---------     | ----------    | ----              |
    | SerialNumberEndpoint  | ()            | u64           | "serial_number"   |
}

#[cfg(test)]
mod test {
    use postcard_rpc::{Endpoint, Key, Topic};

    use super::*;

    fn bytes(key: Key) -> [u8; 8] {
        key.to_bytes()
    }

    /// Each firmware's endpoint list repeats the row for [`SerialNumberEndpoint`]. Same path and schema, so same keys.
    #[test]
    fn serial_number_is_the_same_endpoint_everywhere() {
        for (req, resp) in [
            (
                sensor_actuator::SerialNumberEndpoint::REQ_KEY,
                sensor_actuator::SerialNumberEndpoint::RESP_KEY,
            ),
            (wifi::SerialNumberEndpoint::REQ_KEY, wifi::SerialNumberEndpoint::RESP_KEY),
        ] {
            assert_eq!(bytes(req), bytes(SerialNumberEndpoint::REQ_KEY));
            assert_eq!(bytes(resp), bytes(SerialNumberEndpoint::RESP_KEY));
        }
    }

    /// Devices in the field and the hosted WebUSB app use these keys, so they must not change.
    #[test]
    fn sensor_actuator_keys_are_stable() {
        use sensor_actuator::*;
        let keys: [(&str, Key, [u8; 8]); 8] = [
            (
                "serial_number request",
                SerialNumberEndpoint::REQ_KEY,
                [0x08, 0xda, 0x84, 0xa3, 0xd0, 0x15, 0xde, 0x80],
            ),
            (
                "serial_number response",
                SerialNumberEndpoint::RESP_KEY,
                [0xe4, 0x58, 0x84, 0xa3, 0xd0, 0xc9, 0xdd, 0x80],
            ),
            (
                "actuator/get request",
                GetActuatorEndpoint::REQ_KEY,
                [0x76, 0x93, 0x4c, 0x61, 0x4c, 0xa5, 0xcd, 0xf4],
            ),
            (
                "actuator/get response",
                GetActuatorEndpoint::RESP_KEY,
                [0xc8, 0x09, 0xbc, 0x14, 0xef, 0x73, 0xcb, 0xde],
            ),
            (
                "actuator/set request",
                SetActuatorEndpoint::REQ_KEY,
                [0x44, 0x0c, 0x81, 0xa3, 0x95, 0x9c, 0x10, 0x9d],
            ),
            (
                "actuator/set response",
                SetActuatorEndpoint::RESP_KEY,
                [0x44, 0x0c, 0x81, 0xa3, 0x95, 0x9c, 0x10, 0x9d],
            ),
            (
                "sensor/reading",
                ReadingTopic::TOPIC_KEY,
                [0x3a, 0x70, 0x4c, 0x3b, 0x80, 0x4b, 0xe4, 0xc8],
            ),
            (
                "serial_number request (common)",
                super::SerialNumberEndpoint::REQ_KEY,
                [0x08, 0xda, 0x84, 0xa3, 0xd0, 0x15, 0xde, 0x80],
            ),
        ];
        for (name, key, expected) in keys {
            assert_eq!(bytes(key), expected, "{name}");
        }
    }
}
