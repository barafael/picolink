//! Shared between the server and the UIs.

use dioxus::{
    fullstack::{WebSocketOptions, Websocket},
    prelude::*,
};
use picolink_protocol::sensor_actuator::{ActuatorState, SensorReading};
use serde::{Deserialize, Serialize};
#[cfg(feature = "server")]
use {
    dioxus::{fullstack::TypedWebsocket, server::axum::Extension},
    tokio::sync::{mpsc, watch},
};

/// What the server knows about the device. Pushed to every UI whenever it changes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub enum DeviceState {
    #[default]
    Disconnected,
    Connected {
        /// Where the device is connected, for display.
        connection: String,
        serial_number: u64,
        actuator: ActuatorState,
        /// `None` until the first reading arrived.
        reading: Option<SensorReading>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Command {
    SetActuator(ActuatorState),
}

/// Streams [`DeviceState`] to the UI and forwards its [`Command`]s to the device.
///
/// The server provides both channel ends to the device as router extensions.
#[get("/api/device", state: Extension<watch::Receiver<DeviceState>>, commands: Extension<mpsc::Sender<Command>>)]
pub async fn device_socket(options: WebSocketOptions) -> Result<Websocket<Command, DeviceState>> {
    Ok(options.on_upgrade(|socket| bridge(socket, state.0, commands.0)))
}

/// Runs until the UI or the device goes away.
#[cfg(feature = "server")]
async fn bridge(
    mut socket: TypedWebsocket<Command, DeviceState>,
    mut state: watch::Receiver<DeviceState>,
    commands: mpsc::Sender<Command>,
) {
    state.mark_changed();
    loop {
        tokio::select! {
            changed = state.changed() => {
                if changed.is_err() {
                    break;
                }
                let snapshot = state.borrow_and_update().clone();
                if socket.send(snapshot).await.is_err() {
                    break;
                }
            }
            command = socket.recv() => match command {
                Ok(command) => {
                    if commands.send(command).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            },
        }
    }
}
