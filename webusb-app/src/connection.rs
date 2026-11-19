//! Keeping a page connected to a device.

use std::convert::Infallible;

use leptos::{prelude::*, task::spawn_local};
use postcard_rpc::{Endpoint, postcard_schema::Schema};
use serde::{Serialize, de::DeserializeOwned};

use crate::usb::{self, Client, Device};

/// A page's connection to a device with one USB product string. Cheap to copy.
///
/// The page provides a session: an async function that gets the client, sets up whatever the page shows,
/// calls [`Connection::identified`], and runs for as long as the device is connected.
/// When it fails, the connection is closed and its error shown.
#[derive(Clone, Copy)]
pub struct Connection {
    product: &'static str,
    /// Both hold JS objects and can't leave this thread, hence local storage.
    current: RwSignal<Option<(Device, Client)>, LocalStorage>,
    /// `Some` once the connected device identified itself.
    serial_number: RwSignal<Option<u64>>,
    error: RwSignal<Option<String>>,
}

impl Connection {
    /// `product` is the `USB_PRODUCT` of a module in `picolink_protocol`.
    pub fn new(product: &'static str) -> Self {
        Self {
            product,
            current: RwSignal::new_local(None),
            serial_number: RwSignal::new(None),
            error: RwSignal::new(None),
        }
    }

    /// Connects without the device picker to a device the user granted access to before:
    /// right away if it is plugged in, otherwise as soon as it is. Call once per page.
    pub fn connect_automatically<S, F>(self, session: S)
    where
        S: Fn(Client) -> F + Copy + 'static,
        F: Future<Output = Result<Infallible, String>> + 'static,
    {
        spawn_local(async move {
            match Device::paired(self.product).await {
                Ok(Some(device)) => self.connect(device, session).await,
                Ok(None) => {}
                Err(error) => self.error.set(Some(error)),
            }
        });
        usb::on_connect(self.product, move |device| spawn_local(self.connect(device, session)));
    }

    /// Lets the user pick the device. Only works from a user gesture, such as a click.
    pub async fn connect_picked<S, F>(self, session: S)
    where
        S: Fn(Client) -> F,
        F: Future<Output = Result<Infallible, String>>,
    {
        match Device::request(self.product).await {
            Ok(device) => self.connect(device, session).await,
            Err(error) => self.error.set(Some(error)),
        }
    }

    async fn connect<S, F>(self, device: Device, session: S)
    where
        S: Fn(Client) -> F,
        F: Future<Output = Result<Infallible, String>>,
    {
        if self.current.with_untracked(Option::is_some) {
            return;
        }
        let client = match device.connect().await {
            Ok(client) => client,
            Err(error) => {
                self.error.set(Some(format!("Failed to connect: {error}")));
                return;
            }
        };
        // Left over from an earlier connection.
        self.error.set(None);
        self.current.set(Some((device.clone(), client.clone())));

        let Err(error) = session(client).await;
        // Still the current connection, so this was not a deliberate disconnect.
        if self.is_current(&device) {
            self.disconnect().await;
            self.error.set(Some(format!("Device session ended: {error}")));
        }
    }

    pub async fn disconnect(self) {
        // Cleared first, so that the session knows its failure is expected.
        let Some((device, client)) = self.current.write().take() else {
            return;
        };
        self.serial_number.set(None);
        client.close();
        device.close().await;
    }

    /// A request outside the session, see [`crate::request`].
    ///
    /// `None` without a device, and after a deliberate disconnect: then the request fails because the client was closed,
    /// which is no error worth showing.
    pub async fn request<E>(self, request: &E::Request, timeout_ms: u32) -> Option<Result<E::Response, String>>
    where
        E: Endpoint,
        E::Request: Serialize + Schema,
        E::Response: DeserializeOwned + Schema,
    {
        let (device, client) = self.current.get_untracked()?;
        let result = crate::request::<E>(&client, request, timeout_ms).await;
        (result.is_ok() || self.is_current(&device)).then_some(result)
    }

    /// For the session: the device told its serial number, so the page can show it.
    pub fn identified(&self, serial_number: u64) {
        self.serial_number.set(Some(serial_number));
    }

    /// `Some` once the connected device identified itself. Tracked, for views.
    pub fn serial_number(&self) -> Option<u64> {
        self.serial_number.get()
    }

    /// Tracked, for views.
    pub fn is_connected(&self) -> bool {
        self.current.with(Option::is_some)
    }

    /// Class and text for a status pill. Tracked, for views.
    pub fn status(&self) -> (&'static str, String) {
        match (self.error.get(), self.is_connected(), self.serial_number()) {
            (Some(error), _, _) => ("error", error),
            (None, false, _) => ("pending", "No device".to_owned()),
            (None, true, None) => ("pending", "Connecting…".to_owned()),
            (None, true, Some(_)) => ("ok", "Device connected".to_owned()),
        }
    }

    pub fn set_error(&self, error: Option<String>) {
        self.error.set(error);
    }

    /// Whether `device` is still connected: if not, it was disconnected deliberately.
    fn is_current(&self, device: &Device) -> bool {
        self.current
            .with_untracked(|current| current.as_ref().is_some_and(|(current, _)| current == device))
    }
}
