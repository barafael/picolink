//! Connects to the device, or to the mock firmware, and publishes its [`DeviceState`].

use std::{ops::ControlFlow, pin::pin, time::Duration};

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

/// Where the device is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Usb,
    /// The mock firmware, in this process.
    Mock,
}

/// Owns the connection to the device. The connection itself only lives in the event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Device {
    transport: Transport,
}

impl Device {
    pub fn new(transport: Transport) -> Self {
        Self { transport }
    }

    /// Uses the mock firmware if [`MOCK_FIRMWARE_VAR`] is set.
    pub fn from_env() -> Self {
        Self::new(if std::env::var_os(MOCK_FIRMWARE_VAR).is_some() {
            Transport::Mock
        } else {
            Transport::Usb
        })
    }

    /// Connects, and reconnects whenever the device goes away, until all command senders are gone.
    pub async fn event_loop(self, state: watch::Sender<DeviceState>, mut commands: mpsc::Receiver<Command>) -> Self {
        match self.transport {
            Transport::Usb => info!("Looking for the device"),
            Transport::Mock => info!("Using the mock firmware instead of a device"),
        }
        loop {
            if let Some((client, connection)) = connect(self.transport).await {
                info!(%connection, "Connected");
                let result = session(&client, connection, &state, &mut commands).await;
                client.close();
                state.send_replace(DeviceState::Disconnected);
                match result {
                    Ok(()) => break,
                    Err(SessionError::Disconnected) => info!("Device disconnected"),
                    Err(error) => warn!(error = &error as &dyn std::error::Error, "Device session ended"),
                }
            }
            if idle(&mut commands).await.is_break() {
                break;
            }
        }
        info!("No more commands, stopping");
        self
    }
}

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

/// Returns the client and a description of the connection.
async fn connect(transport: Transport) -> Option<(Client, String)> {
    if transport == Transport::Mock {
        let (client, firmware) = picolink_mock_firmware::in_process();
        // Stops once the client is closed.
        tokio::spawn(firmware);
        return Some((client, "mock firmware".to_owned()));
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

/// Waits before looking for the device again. There is no device to take commands meanwhile, so they are dropped.
///
/// Breaks once all command senders are gone.
async fn idle(commands: &mut mpsc::Receiver<Command>) -> ControlFlow<()> {
    let mut scan = pin!(sleep(SCAN_INTERVAL));
    loop {
        tokio::select! {
            () = &mut scan => return ControlFlow::Continue(()),
            command = commands.recv() => match command {
                Some(command) => warn!(?command, "No device, dropping command"),
                None => return ControlFlow::Break(()),
            },
        }
    }
}

/// Runs until the device goes away or misbehaves, or until all command senders are gone, which is `Ok`.
async fn session(
    client: &Client,
    connection: String,
    state: &watch::Sender<DeviceState>,
    commands: &mut mpsc::Receiver<Command>,
) -> Result<(), SessionError> {
    let serial_number = request::<SerialNumberEndpoint>(client, &()).await?;
    let actuator = request::<GetActuatorEndpoint>(client, &()).await?;
    let mut readings = client.subscribe_multi::<ReadingTopic>(READING_DEPTH).await?;
    info!(serial_number = %format_args!("{serial_number:016x}"), "Device identified");

    // Sent while connecting, so not meant for this device. Before announcing it, so that later commands are kept.
    while commands.try_recv().is_ok() {}
    state.send_replace(DeviceState::Connected {
        connection,
        serial_number,
        actuator,
        reading: None,
    });

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
            command = commands.recv() => match command {
                Some(Command::SetActuator(new)) => {
                    let new = request::<SetActuatorEndpoint>(client, &new).await?;
                    state.send_modify(|state| {
                        if let DeviceState::Connected { actuator, .. } = state {
                            *actuator = new;
                        }
                    });
                }
                None => return Ok(()),
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

#[cfg(test)]
mod test {
    use picolink_protocol::ActuatorState;

    use super::*;

    fn actuator_is(expected: ActuatorState) -> impl FnMut(&DeviceState) -> bool {
        move |state| matches!(state, DeviceState::Connected { actuator, .. } if *actuator == expected)
    }

    #[tokio::test]
    async fn sets_the_actuator_then_stops_without_senders() {
        let device = Device::new(Transport::Mock);
        let (state_tx, mut state) = watch::channel(DeviceState::Disconnected);
        let (commands, commands_rx) = mpsc::channel(1);
        let event_loop = tokio::spawn(device.event_loop(state_tx, commands_rx));

        state.wait_for(actuator_is(ActuatorState::Off)).await.unwrap();
        commands.send(Command::SetActuator(ActuatorState::On)).await.unwrap();
        state.wait_for(actuator_is(ActuatorState::On)).await.unwrap();

        drop(commands);
        assert_eq!(event_loop.await.unwrap(), device);
        assert_eq!(*state.borrow(), DeviceState::Disconnected);
    }
}
