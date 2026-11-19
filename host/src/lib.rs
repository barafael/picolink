//! The host side of picolink: finding a device over USB, connecting to it, and making requests that time out.
//!
//! What to send is up to the caller, from the device's module in `picolink_protocol`:
//!
//! ```no_run
//! use picolink_host::{DEFAULT_TIMEOUT, DeviceFilter, connect, request};
//! use picolink_protocol::sensor_actuator::{GetActuatorEndpoint, USB_PRODUCT};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let device = connect(&DeviceFilter::new(USB_PRODUCT)).await?;
//! let actuator = request::<GetActuatorEndpoint>(&device.client, &(), DEFAULT_TIMEOUT).await?;
//! # Ok(())
//! # }
//! ```

use std::{fmt, time::Duration};

use picolink_protocol::{SerialNumberEndpoint, USB_PID, USB_VID};
use postcard_rpc::{
    Endpoint,
    header::VarSeqKind,
    host_client::{HostClient, HostErr, IoClosed},
    postcard_schema::Schema,
    standard_icd::{ERROR_PATH, WireError},
};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

pub type Client = HostClient<WireError>;

/// Long enough for any request that the device answers right away.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

/// Outgoing request queue.
const CLIENT_DEPTH: usize = 8;

/// Which device to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFilter {
    product: &'static str,
    serial_number: Option<String>,
}

impl DeviceFilter {
    /// Devices with this USB product string, the `USB_PRODUCT` of a module in `picolink_protocol`.
    ///
    /// The VID and PID are shared with the MT21, a serial device that must not be claimed, so this is required.
    pub fn new(product: &'static str) -> Self {
        Self {
            product,
            serial_number: None,
        }
    }

    /// Only the device with this serial number, as [`SerialNumberEndpoint`] answers it.
    #[must_use]
    pub fn serial_number(mut self, serial_number: u64) -> Self {
        // The firmware's USB serial number string.
        self.serial_number = Some(format!("{serial_number:016x}"));
        self
    }

    pub fn matches(&self, device: &nusb::DeviceInfo) -> bool {
        device.vendor_id() == USB_VID
            && device.product_id() == USB_PID
            && device.product_string() == Some(self.product)
            && self
                .serial_number
                .as_deref()
                .is_none_or(|serial| device.serial_number() == Some(serial))
    }
}

/// A connected device.
pub struct Device {
    pub client: Client,
    pub location: Location,
}

/// Where a device is connected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub bus_id: String,
    pub address: u8,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "USB bus {}, address {}", self.bus_id, self.address)
    }
}

#[derive(Debug, Error)]
pub enum ConnectError {
    #[error("failed to list USB devices")]
    List(#[source] nusb::Error),
    #[error("no matching device")]
    NotFound,
    /// For example because another program has claimed it.
    #[error("failed to open the device: {0}")]
    Open(String),
}

/// Connects to the first device that matches `filter`.
pub async fn connect(filter: &DeviceFilter) -> Result<Device, ConnectError> {
    let device = nusb::list_devices()
        .await
        .map_err(ConnectError::List)?
        .find(|device| filter.matches(device))
        .ok_or(ConnectError::NotFound)?;
    let location = Location {
        bus_id: device.bus_id().to_owned(),
        address: device.device_address(),
    };
    let client = HostClient::try_new_raw_nusb(|device| filter.matches(device), ERROR_PATH, CLIENT_DEPTH, VarSeqKind::Seq4)
        .await
        .map_err(ConnectError::Open)?;
    Ok(Device { client, location })
}

/// Why a request failed.
#[derive(Debug, Error)]
pub enum RequestError {
    #[error("device disconnected")]
    Disconnected,
    #[error("no response on {path:?} within {timeout:?}")]
    Timeout { path: &'static str, timeout: Duration },
    /// Typically because the device speaks another protocol, or another version of it.
    #[error("device rejected the request on {path:?}")]
    Rejected {
        path: &'static str,
        #[source]
        source: WireError,
    },
    #[error("invalid response on {path:?}")]
    InvalidResponse {
        path: &'static str,
        #[source]
        source: HostErr<WireError>,
    },
}

impl RequestError {
    fn from_host(path: &'static str, error: HostErr<WireError>) -> Self {
        match error {
            HostErr::Closed => Self::Disconnected,
            HostErr::Wire(source) => Self::Rejected { path, source },
            source @ (HostErr::BadResponse | HostErr::Postcard(_)) => Self::InvalidResponse { path, source },
        }
    }
}

/// Subscribing fails the same way.
impl From<IoClosed> for RequestError {
    fn from(IoClosed: IoClosed) -> Self {
        Self::Disconnected
    }
}

/// A request that gives up after `timeout` instead of waiting forever for a response that got lost.
pub async fn request<E>(client: &Client, request: &E::Request, timeout: Duration) -> Result<E::Response, RequestError>
where
    E: Endpoint,
    E::Request: Serialize + Schema,
    E::Response: DeserializeOwned + Schema,
{
    tokio::time::timeout(timeout, client.send_resp::<E>(request))
        .await
        .map_err(|_| RequestError::Timeout { path: E::PATH, timeout })?
        .map_err(|error| RequestError::from_host(E::PATH, error))
}

/// Every picolink firmware answers this.
pub async fn serial_number(client: &Client) -> Result<u64, RequestError> {
    request::<SerialNumberEndpoint>(client, &(), DEFAULT_TIMEOUT).await
}

#[cfg(test)]
mod test {
    use picolink_protocol::{sensor_actuator, wifi};

    use super::*;

    #[test]
    fn serial_number_filter_uses_the_firmware_format() {
        let filter = DeviceFilter::new(sensor_actuator::USB_PRODUCT).serial_number(0xe661_4104_0354_6f38);
        assert_eq!(filter.serial_number.as_deref(), Some("e661410403546f38"));
        assert_ne!(filter, DeviceFilter::new(wifi::USB_PRODUCT).serial_number(0xe661_4104_0354_6f38));
    }

    #[tokio::test]
    async fn serial_number_from_any_firmware() {
        let (client, firmware) = picolink_mock_firmware::in_process();
        let firmware = tokio::spawn(firmware);
        assert_eq!(serial_number(&client).await.unwrap(), picolink_mock_firmware::SERIAL_NUMBER);
        client.close();
        firmware.await.unwrap();
    }

    #[tokio::test]
    async fn requests_time_out() {
        // A device that never answers: the other channel ends stay open, but nothing reads or writes them.
        let (client_tx, _server_rx) = tokio::sync::mpsc::channel(4);
        let (_server_tx, client_rx) = tokio::sync::mpsc::channel(4);
        let client = postcard_rpc::host_client::test_channels::new_from_channels(client_tx, client_rx, VarSeqKind::Seq4);
        let timeout = Duration::from_millis(50);
        let error = request::<SerialNumberEndpoint>(&client, &(), timeout).await.unwrap_err();
        assert!(
            matches!(error, RequestError::Timeout { path: "serial_number", timeout: t } if t == timeout),
            "{error:?}"
        );
    }
}
