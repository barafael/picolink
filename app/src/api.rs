//! Shared between the server and the UIs.

use dioxus::{
    fullstack::{WebSocketOptions, Websocket},
    prelude::*,
};
use picolink_protocol::{ActuatorState, SensorReading};
use serde::{Deserialize, Serialize};
#[cfg(feature = "server")]
use {crate::device::DeviceHandle, dioxus::fullstack::TypedWebsocket};

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
#[get("/api/device")]
pub async fn device_socket(options: WebSocketOptions) -> Result<Websocket<Command, DeviceState>> {
    Ok(options.on_upgrade(|socket| bridge(socket, crate::device::handle())))
}

/// Runs until the UI goes away.
#[cfg(feature = "server")]
async fn bridge(mut socket: TypedWebsocket<Command, DeviceState>, device: &'static DeviceHandle) {
    let mut state = device.state();
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
                Ok(command) => device.command(command),
                Err(_) => break,
            },
        }
    }
}
