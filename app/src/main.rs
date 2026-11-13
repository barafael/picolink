//! The server owns the connection to the device and serves the UI.
//! UIs (web or desktop) talk to the server over a websocket, see [`api`].

mod api;
#[cfg(feature = "server")]
mod device;
mod ui;

#[cfg(feature = "server")]
use {
    api::DeviceState,
    device::Device,
    dioxus::server::axum::{Extension, Router},
    tokio::sync::{mpsc, watch},
};

/// Commands from all UIs, waiting for the device.
#[cfg(feature = "server")]
const COMMAND_DEPTH: usize = 16;

fn main() {
    #[cfg(feature = "server")]
    dioxus::serve(router);

    #[cfg(not(feature = "server"))]
    dioxus::launch(ui::App);
}

/// Can't fail, hence any error type.
#[cfg(feature = "server")]
async fn router<E>() -> Result<Router, E> {
    let (state_tx, state) = watch::channel(DeviceState::Disconnected);
    let (commands, commands_rx) = mpsc::channel(COMMAND_DEPTH);
    // Look for the device right away, not only once a UI connects.
    tokio::spawn(Device::from_env().event_loop(state_tx, commands_rx));
    Ok(dioxus::server::router(ui::App).layer(Extension(state)).layer(Extension(commands)))
}
