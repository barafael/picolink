//! Connects to the device, or to the mock firmware, and publishes its [`DeviceState`].

use std::{ops::ControlFlow, pin::pin, time::Duration};

use dioxus::logger::tracing::{info, warn};
use picolink_host::{Client, ConnectError, DEFAULT_TIMEOUT, DeviceFilter, RequestError, request, serial_number};
use picolink_protocol::sensor_actuator::{GetActuatorEndpoint, ReadingTopic, SetActuatorEndpoint, USB_PRODUCT};
use postcard_rpc::host_client::MultiSubRxError;
use tokio::{
    sync::{mpsc, watch},
    time::sleep,
};

use crate::api::{Command, DeviceState};

/// Set to use the mock firmware instead of a real device.
const MOCK_FIRMWARE_VAR: &str = "PICOLINK_MOCK_FIRMWARE";

const SCAN_INTERVAL: Duration = Duration::from_secs(1);
const READING_DEPTH: usize = 16;

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
                    Err(RequestError::Disconnected) => info!("Device disconnected"),
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

/// Returns the client and a description of the connection.
async fn connect(transport: Transport) -> Option<(Client, String)> {
    if transport == Transport::Mock {
        let (client, firmware) = picolink_mock_firmware::in_process();
        // Stops once the client is closed.
        tokio::spawn(firmware);
        return Some((client, "mock firmware".to_owned()));
    }
    match picolink_host::connect(&DeviceFilter::new(USB_PRODUCT)).await {
        Ok(device) => Some((device.client, device.location.to_string())),
        // The usual case while nothing is plugged in.
        Err(ConnectError::NotFound) => None,
        Err(error) => {
            warn!(error = &error as &dyn std::error::Error, "Failed to connect");
            None
        }
    }
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
) -> Result<(), RequestError> {
    let serial_number = serial_number(client).await?;
    let actuator = request::<GetActuatorEndpoint>(client, &(), DEFAULT_TIMEOUT).await?;
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
            () = client.wait_closed() => return Err(RequestError::Disconnected),
            reading = readings.recv() => match reading {
                Ok(new) => state.send_modify(|state| {
                    if let DeviceState::Connected { reading, .. } = state {
                        *reading = Some(new);
                    }
                }),
                Err(MultiSubRxError::Lagged(missed)) => warn!(missed, "Missed readings"),
                Err(MultiSubRxError::IoClosed) => return Err(RequestError::Disconnected),
            },
            command = commands.recv() => match command {
                Some(Command::SetActuator(new)) => {
                    let new = request::<SetActuatorEndpoint>(client, &new, DEFAULT_TIMEOUT).await?;
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

#[cfg(test)]
mod test {
    use picolink_protocol::sensor_actuator::ActuatorState;

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
