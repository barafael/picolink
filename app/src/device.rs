//! Connects to the device, or to the mock firmware, and keeps [`DeviceState`] up to date.

use std::{convert::Infallible, sync::LazyLock, time::Duration};

use dioxus::logger::tracing::{info, warn};
use picolink_protocol::{GetActuatorEndpoint, ReadingTopic, SerialNumberEndpoint, SetActuatorEndpoint, USB_PID, USB_PRODUCT, USB_VID};
use postcard_rpc::{
    Endpoint,
    header::VarSeqKind,
    host_client::{HostClient, HostErr, IoClosed, MultiSubRxError},
    postcard_schema::Schema,
    standard_icd::{ERROR_PATH, WireError},
};
use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;
use tokio::{
    sync::{mpsc, watch},
    time::{sleep, timeout},
};

use crate::api::{Command, DeviceState};

/// Set to use the mock firmware instead of a real device.
const MOCK_FIRMWARE_VAR: &str = "PICOLINK_MOCK_FIRMWARE";

const SCAN_INTERVAL: Duration = Duration::from_secs(1);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);
/// Outgoing request queue.
const CLIENT_DEPTH: usize = 8;
const READING_DEPTH: usize = 16;

type Client = HostClient<WireError>;

/// Why a device session ended.
#[derive(Debug, Error)]
enum SessionError {
    #[error("device disconnected")]
    Disconnected,
    #[error("no response on {path:?} within {REQUEST_TIMEOUT:?}")]
    Timeout { path: &'static str },
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

impl SessionError {
    fn from_host(path: &'static str, error: HostErr<WireError>) -> Self {
        match error {
            HostErr::Closed => Self::Disconnected,
            HostErr::Wire(source) => Self::Rejected { path, source },
            source @ (HostErr::BadResponse | HostErr::Postcard(_)) => Self::InvalidResponse { path, source },
        }
    }
}

impl From<IoClosed> for SessionError {
    fn from(IoClosed: IoClosed) -> Self {
        Self::Disconnected
    }
}

static DEVICE: LazyLock<DeviceHandle> = LazyLock::new(|| {
    let (state_tx, state) = watch::channel(DeviceState::Disconnected);
    let (commands, commands_rx) = mpsc::channel(16);
    tokio::spawn(run(state_tx, commands_rx));
    DeviceHandle { state, commands }
});

/// Starts the device task. Must be called inside the tokio runtime.
pub fn start() {
    LazyLock::force(&DEVICE);
}

pub fn handle() -> &'static DeviceHandle {
    &DEVICE
}

pub struct DeviceHandle {
    state: watch::Receiver<DeviceState>,
    commands: mpsc::Sender<Command>,
}

impl DeviceHandle {
    pub fn state(&self) -> watch::Receiver<DeviceState> {
        self.state.clone()
    }

    pub fn command(&self, command: Command) {
        if let Err(error) = self.commands.try_send(command) {
            warn!(%error, ?command, "Dropping command");
        }
    }
}

async fn run(state: watch::Sender<DeviceState>, mut commands: mpsc::Receiver<Command>) {
    let mock = std::env::var_os(MOCK_FIRMWARE_VAR).is_some();
    if mock {
        info!("Using the mock firmware instead of a device");
    } else {
        info!("Looking for the device");
    }
    loop {
        if let Some((client, connection)) = connect(mock).await {
            info!(%connection, "Connected");
            let result = session(&client, connection, &state, &mut commands).await;
            client.close();
            state.send_replace(DeviceState::Disconnected);
            match result {
                Err(SessionError::Disconnected) => info!("Device disconnected"),
                Err(error) => warn!(error = &error as &dyn std::error::Error, "Device session ended"),
            }
        }
        sleep(SCAN_INTERVAL).await;
    }
}

/// Returns the client and a description of the connection.
async fn connect(mock: bool) -> Option<(Client, String)> {
    if mock {
        return Some((picolink_mock_firmware::spawn(), "mock firmware".to_owned()));
    }
    let mut devices = nusb::list_devices()
        .await
        .inspect_err(|error| warn!(%error, "Failed to list USB devices"))
        .ok()?;
    let device = devices.find(is_picolink)?;
    let connection = format!("USB bus {}, address {}", device.bus_id(), device.device_address());
    let client = HostClient::try_new_raw_nusb(is_picolink, ERROR_PATH, CLIENT_DEPTH, VarSeqKind::Seq4)
        .await
        .inspect_err(|error| warn!(%error, "Failed to open the device"))
        .ok()?;
    Some((client, connection))
}

fn is_picolink(device: &nusb::DeviceInfo) -> bool {
    // The MT21 shares VID and PID. It is a serial device and must not be claimed.
    device.vendor_id() == USB_VID && device.product_id() == USB_PID && device.product_string() == Some(USB_PRODUCT)
}

/// Runs until the device goes away or misbehaves.
async fn session(
    client: &Client,
    connection: String,
    state: &watch::Sender<DeviceState>,
    commands: &mut mpsc::Receiver<Command>,
) -> Result<Infallible, SessionError> {
    let serial_number = request::<SerialNumberEndpoint>(client, &()).await?;
    let actuator = request::<GetActuatorEndpoint>(client, &()).await?;
    let mut readings = client.subscribe_multi::<ReadingTopic>(READING_DEPTH).await?;
    info!(serial_number = %format_args!("{serial_number:016x}"), "Device identified");
    state.send_replace(DeviceState::Connected {
        connection,
        serial_number,
        actuator,
        reading: None,
    });

    // Commands sent while no device was connected must not reach this one.
    while commands.try_recv().is_ok() {}

    loop {
        tokio::select! {
            () = client.wait_closed() => return Err(SessionError::Disconnected),
            reading = readings.recv() => match reading {
                Ok(new) => state.send_modify(|state| {
                    if let DeviceState::Connected { reading, .. } = state {
                        *reading = Some(new);
                    }
                }),
                Err(MultiSubRxError::Lagged(missed)) => warn!(missed, "Missed readings"),
                Err(MultiSubRxError::IoClosed) => return Err(SessionError::Disconnected),
            },
            Some(command) = commands.recv() => match command {
                Command::SetActuator(new) => {
                    let new = request::<SetActuatorEndpoint>(client, &new).await?;
                    state.send_modify(|state| {
                        if let DeviceState::Connected { actuator, .. } = state {
                            *actuator = new;
                        }
                    });
                }
            },
        }
    }
}

/// A request that gives up instead of waiting forever for a response.
async fn request<E>(client: &Client, request: &E::Request) -> Result<E::Response, SessionError>
where
    E: Endpoint,
    E::Request: Serialize + Schema,
    E::Response: DeserializeOwned + Schema,
{
    timeout(REQUEST_TIMEOUT, client.send_resp::<E>(request))
        .await
        .map_err(|_| SessionError::Timeout { path: E::PATH })?
        .map_err(|error| SessionError::from_host(E::PATH, error))
}
