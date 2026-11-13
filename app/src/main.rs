//! The server owns the serial connection to the device and serves the UI.
//! UIs (web or desktop) talk to the server over a websocket, see [`api`].

mod api;
#[cfg(feature = "server")]
mod device;
mod ui;

fn main() {
    #[cfg(feature = "server")]
    dioxus::serve(router);

    #[cfg(not(feature = "server"))]
    dioxus::launch(ui::App);
}

/// Can't fail, hence any error type.
#[cfg(feature = "server")]
async fn router<E>() -> Result<dioxus::server::axum::Router, E> {
    // Look for the device right away, not only once a UI connects.
    device::start();
    Ok(dioxus::server::router(ui::App))
}
