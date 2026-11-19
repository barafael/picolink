//! WebUSB building blocks for picolink pages: the browser talks to a device directly, no server involved.
//!
//! - [`usb`]: finding a device by its USB product string and connecting postcard-rpc's WebUSB client to it.
//! - [`connection`]: keeping a Leptos page connected, reconnecting when the device is plugged in again.
//! - [`page`]: the frame every page shares, with the status pill and the button to connect.
//! - [`request`]: requests that time out.
//!
//! The pages in `src/bin/` are built from these, one per firmware:
//! - `sensor_actuator`: the same UI as `app/`, without a server.
//! - `wifi`: the Wi-Fi networks a Pico W sees, and joining one.
//!
//! postcard-rpc's WebUSB client reads one USB packet per transfer, so every message the device sends must fit in 64 bytes.

use std::pin::pin;

use futures_util::future::{Either, select};
use gloo_timers::future::TimeoutFuture;
use postcard_rpc::{Endpoint, postcard_schema::Schema};
use serde::{Serialize, de::DeserializeOwned};

pub mod connection;
pub mod page;
pub mod usb;

pub use crate::{connection::Connection, page::Page, usb::Client};

/// Long enough for any request that the device answers right away. Same as the host library's.
pub const DEFAULT_TIMEOUT_MS: u32 = 1000;

/// Gives up after `timeout_ms` instead of waiting forever for a response that got lost.
pub async fn request<E>(client: &Client, request: &E::Request, timeout_ms: u32) -> Result<E::Response, String>
where
    E: Endpoint,
    E::Request: Serialize + Schema,
    E::Response: DeserializeOwned + Schema,
{
    match select(pin!(client.send_resp::<E>(request)), TimeoutFuture::new(timeout_ms)).await {
        Either::Left((response, _)) => response.map_err(|error| format!("{error:?}")),
        Either::Right(((), _)) => Err(format!("No response on {:?} within {timeout_ms} ms", E::PATH)),
    }
}
