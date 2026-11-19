//! A Pico W that keeps scanning for Wi-Fi networks, and joins one on request.
//!
//! The device scans continuously and streams each scan on [`ScanTopic`]: one [`ScanEvent::Network`] per network,
//! then [`ScanEvent::Done`]. One network per message keeps every message the device sends within one 64-byte USB packet,
//! which postcard-rpc's WebUSB client needs: it reads one packet per transfer.

use heapless::String;
use postcard_rpc::{TopicDirection, endpoints, topics};
use postcard_schema::Schema;
use serde::{Deserialize, Serialize};

/// Hosts that don't speak this module's protocol, like the server, ignore devices with this USB product string.
pub const USB_PRODUCT: &str = "picolink-wifi";

/// Longest SSID, as in IEEE 802.11.
pub const SSID_LEN: usize = 32;
/// WPA passphrases have 8 to 63 characters. 64 characters would be a raw hex key, which is not supported.
pub const PASSPHRASE_LEN: core::ops::RangeInclusive<usize> = 8..=63;
/// Most networks a scan reports. Further ones are dropped, the weakest first.
pub const MAX_NETWORKS: usize = 20;

/// After this long, a scan reports what it found so far.
pub const SCAN_TIMEOUT_MS: u64 = 10_000;
/// After this long, joining fails with [`JoinError::Timeout`].
pub const JOIN_TIMEOUT_MS: u64 = 20_000;
/// How long a host should wait for the answer to [`JoinEndpoint`]: the device may have to finish a scan first.
pub const JOIN_ANSWER_TIMEOUT_MS: u64 = SCAN_TIMEOUT_MS + JOIN_TIMEOUT_MS + 2_000;
/// How long a host should wait for the answer to [`LeaveEndpoint`], for the same reason.
pub const LEAVE_ANSWER_TIMEOUT_MS: u64 = SCAN_TIMEOUT_MS + 2_000;

pub type Ssid = String<SSID_LEN>;
pub type Passphrase = String<{ *PASSPHRASE_LEN.end() }>;
pub type JoinResult = Result<(), JoinError>;

endpoints! {
    list = ENDPOINT_LIST;
    | EndpointTy            | RequestTy     | ResponseTy    | Path              |
    | ----------            | ---------     | ----------    | ----              |
    | SerialNumberEndpoint  | ()            | u64           | "serial_number"   |
    | StatusEndpoint        | ()            | Status        | "wifi/status"     |
    | JoinEndpoint          | JoinRequest   | JoinResult    | "wifi/join"       |
    | LeaveEndpoint         | ()            | ()            | "wifi/leave"      |
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
    | ScanTopic             | ScanEvent     | "wifi/scan"       |
}

/// A network found by a scan. Networks whose SSID is not UTF-8 are left out.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Schema)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Network {
    /// Empty for hidden networks.
    pub ssid: Ssid,
    pub bssid: [u8; 6],
    pub channel: u8,
    /// Signal strength in dBm. Of all reports for this BSSID during the scan, the strongest.
    pub rssi: i16,
    /// Whether joining needs a passphrase.
    pub secured: bool,
}

/// Part of a scan. `scan` counts scans, so that a host can tell which networks belong together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum ScanEvent {
    /// Sent strongest first.
    Network { scan: u16, network: Network },
    /// The scan is complete. `networks` is how many [`ScanEvent::Network`]s it sent: fewer arrived if some got lost.
    Done { scan: u16, networks: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Status {
    /// The network joined last, unless left since. The device doesn't notice when the network goes away.
    pub joined: Option<Ssid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct JoinRequest {
    pub ssid: Ssid,
    /// `None` for open networks. Otherwise WPA2 or WPA3.
    pub passphrase: Option<Passphrase>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum JoinError {
    NetworkNotFound,
    /// Usually a wrong passphrase.
    AuthenticationFailed,
    /// Any other failure, with the status code of the Wi-Fi chip.
    Failed(u8),
    /// No answer within [`JOIN_TIMEOUT_MS`].
    Timeout,
}

#[cfg(test)]
mod test {
    use super::*;

    /// A postcard-rpc header is at most this long: flags, an 8-byte key and a 4-byte sequence number.
    const MAX_HEADER: usize = 1 + 8 + 4;
    /// One full-speed USB packet.
    const MAX_MESSAGE: usize = 64;

    fn fits_one_packet<T: Serialize>(name: &str, message: &T) {
        let mut buffer = [0; 128];
        let len = postcard::to_slice(message, &mut buffer).unwrap().len();
        assert!(MAX_HEADER + len <= MAX_MESSAGE, "{name}: {MAX_HEADER} + {len} bytes");
    }

    #[test]
    fn device_messages_fit_one_usb_packet() {
        let longest_ssid = Ssid::try_from("x".repeat(SSID_LEN).as_str()).unwrap();
        let network = Network {
            ssid: longest_ssid.clone(),
            bssid: [0xFF; 6],
            channel: u8::MAX,
            rssi: i16::MIN,
            secured: true,
        };
        fits_one_packet("ScanEvent::Network", &ScanEvent::Network { scan: u16::MAX, network });
        fits_one_packet(
            "ScanEvent::Done",
            &ScanEvent::Done {
                scan: u16::MAX,
                networks: u8::MAX,
            },
        );
        fits_one_packet(
            "Status",
            &Status {
                joined: Some(longest_ssid),
            },
        );
        fits_one_packet("JoinResult", &JoinResult::Err(JoinError::Failed(u8::MAX)));
        fits_one_packet("serial number", &u64::MAX);
    }
}
