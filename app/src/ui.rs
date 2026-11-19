use std::time::Duration;

use dioxus::{
    fullstack::{UseWebsocket, WebSocketOptions, use_websocket},
    prelude::*,
};
use picolink_protocol::sensor_actuator::{ActuatorState, SensorReading};

use crate::api::{Command, DeviceState, device_socket};

const MAIN_CSS: Asset = asset!("/assets/main.css");

const RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, PartialEq)]
enum Server {
    Connecting,
    Connected,
    Unreachable,
}

#[component]
pub fn App() -> Element {
    let socket = use_websocket(|| device_socket(WebSocketOptions::new()));
    // Not the socket's status: in browsers, opening a websocket succeeds before the server answered.
    let server = use_signal(|| Server::Connecting);
    let device = use_signal(DeviceState::default);

    use_future(move || follow_device(socket, server, device));

    let (class, status) = match server() {
        Server::Connecting => ("pending", "Connecting to server…"),
        Server::Connected => ("ok", "Server connected"),
        Server::Unreachable => ("error", "Server unreachable"),
    };

    rsx! {
        document::Title { "picolink" }
        document::Stylesheet { href: MAIN_CSS }
        header {
            h1 { "picolink" }
            span { class: "pill {class}", {status} }
        }
        main {
            if let DeviceState::Connected { connection, serial_number, actuator, reading } = device() {
                DeviceCard { connection, serial_number }
                ActuatorCard {
                    state: actuator,
                    onchange: move |state| set_actuator(socket, state),
                }
                SensorCard { reading }
            } else {
                p { class: "hint", "Waiting for the device…" }
            }
        }
    }
}

/// Mirrors the server's [`DeviceState`], reconnecting whenever the socket drops.
async fn follow_device(mut socket: UseWebsocket<Command, DeviceState>, mut server: Signal<Server>, mut device: Signal<DeviceState>) {
    loop {
        // The server sends the current state right away, so this confirms the connection.
        while let Ok(state) = socket.recv().await {
            server.set(Server::Connected);
            device.set(state);
        }
        // Without the server, nothing is known about the device.
        server.set(Server::Unreachable);
        device.set(DeviceState::Disconnected);
        sleep(RECONNECT_DELAY).await;
        socket.set(device_socket(WebSocketOptions::new()).await);
    }
}

async fn set_actuator(socket: UseWebsocket<Command, DeviceState>, state: ActuatorState) {
    _ = socket.send(Command::SetActuator(state)).await;
}

#[component]
fn DeviceCard(connection: String, serial_number: u64) -> Element {
    rsx! {
        section { class: "card",
            h2 { "Device" }
            dl {
                dt { "Connection" }
                dd { {connection} }
                dt { "Serial number" }
                dd { "{serial_number:016x}" }
            }
        }
    }
}

#[component]
fn ActuatorCard(state: ActuatorState, onchange: EventHandler<ActuatorState>) -> Element {
    let label = match state {
        ActuatorState::On => "On",
        ActuatorState::Off => "Off",
    };
    rsx! {
        section { class: "card",
            h2 { "Actuator" }
            p { class: "label", "Onboard LED" }
            button {
                class: "toggle",
                class: if state == ActuatorState::On { "on" },
                onclick: move |_| onchange.call(state.toggled()),
                {label}
            }
        }
    }
}

#[component]
fn SensorCard(reading: Option<SensorReading>) -> Element {
    rsx! {
        section { class: "card",
            h2 { "Sensor" }
            p { class: "label", "Chip temperature" }
            p { class: "value",
                if let Some(reading) = reading {
                    "{reading.celsius:.1} °C"
                } else {
                    "…"
                }
            }
        }
    }
}

async fn sleep(duration: Duration) {
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::sleep(duration).await;
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(duration).await;
}
