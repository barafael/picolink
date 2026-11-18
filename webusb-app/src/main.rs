//! The browser talks to the device directly through WebUSB, no server involved.
//! The alternative to `app/`, with the same protocol and the same UI.

mod usb;

use std::{convert::Infallible, pin::pin};

use futures_util::future::{Either, select};
use gloo_timers::future::TimeoutFuture;
use leptos::{prelude::*, task::spawn_local};
use picolink_protocol::{ActuatorState, GetActuatorEndpoint, ReadingTopic, SensorReading, SerialNumberEndpoint, SetActuatorEndpoint};
use postcard_rpc::{
    Endpoint,
    host_client::{HostErr, MultiSubRxError},
    standard_icd::WireError,
};

use crate::usb::{Client, Picolink};

const READING_DEPTH: usize = 16;
/// Same as the server's.
const REQUEST_TIMEOUT_MS: u32 = 1000;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[derive(Clone, Copy)]
struct State {
    /// Both hold JS objects and can't leave this thread, hence local storage.
    connection: RwSignal<Option<(Picolink, Client)>, LocalStorage>,
    error: RwSignal<Option<String>>,
    /// `Some` once the device identified itself.
    serial_number: RwSignal<Option<u64>>,
    actuator: RwSignal<ActuatorState>,
    reading: RwSignal<Option<SensorReading>>,
}

impl State {
    fn new() -> Self {
        Self {
            connection: RwSignal::new_local(None),
            error: RwSignal::new(None),
            serial_number: RwSignal::new(None),
            actuator: RwSignal::new(ActuatorState::Off),
            reading: RwSignal::new(None),
        }
    }

    /// Reconnects without the device picker to a device the user granted access to before, if it is plugged in.
    async fn connect_paired(self) {
        match Picolink::paired().await {
            Ok(Some(picolink)) => self.connect(picolink).await,
            Ok(None) => {}
            Err(error) => self.error.set(Some(error)),
        }
    }

    /// Lets the user pick the device.
    async fn connect_picked(self) {
        match Picolink::request().await {
            Ok(picolink) => self.connect(picolink).await,
            Err(error) => self.error.set(Some(error)),
        }
    }

    async fn connect(self, picolink: Picolink) {
        if self.connection.with_untracked(Option::is_some) {
            return;
        }
        let client = match picolink.connect().await {
            Ok(client) => client,
            Err(error) => {
                self.error.set(Some(format!("Failed to connect: {error}")));
                return;
            }
        };
        // Leftovers from an earlier connection.
        self.error.set(None);
        self.serial_number.set(None);
        self.reading.set(None);
        self.connection.set(Some((picolink.clone(), client.clone())));

        let Err(error) = self.session(&client).await;
        // Still the current connection, so this was not a deliberate disconnect.
        if self
            .connection
            .with_untracked(|current| current.as_ref().is_some_and(|(current, _)| *current == picolink))
        {
            self.disconnect().await;
            self.error.set(Some(format!("Device session ended: {error}")));
        }
    }

    async fn disconnect(self) {
        // Cleared first, so that the session knows its failure is expected.
        let Some((picolink, client)) = self.connection.write().take() else {
            return;
        };
        self.serial_number.set(None);
        client.close();
        picolink.close().await;
    }

    /// Runs for as long as the device is connected.
    async fn session(self, client: &Client) -> Result<Infallible, String> {
        // Protocol version mismatches show up here: the device rejects requests it doesn't know.
        let serial_number = request::<SerialNumberEndpoint>(client.send_resp::<SerialNumberEndpoint>(&())).await?;
        let actuator = request::<GetActuatorEndpoint>(client.send_resp::<GetActuatorEndpoint>(&())).await?;
        let mut readings = client
            .subscribe_multi::<ReadingTopic>(READING_DEPTH)
            .await
            .map_err(|_| "Client closed")?;
        self.actuator.set(actuator);
        self.serial_number.set(Some(serial_number));
        loop {
            match readings.recv().await {
                Ok(reading) => self.reading.set(Some(reading)),
                Err(MultiSubRxError::Lagged(_)) => {}
                Err(MultiSubRxError::IoClosed) => return Err("Device disconnected".into()),
            }
        }
    }

    fn set_actuator(self, state: ActuatorState) {
        let Some((_, client)) = self.connection.get_untracked() else {
            return;
        };
        spawn_local(self.send_actuator(client, state));
    }

    async fn send_actuator(self, client: Client, state: ActuatorState) {
        match request::<SetActuatorEndpoint>(client.send_resp::<SetActuatorEndpoint>(&state)).await {
            Ok(state) => {
                self.actuator.set(state);
                // An earlier request may have failed, but the device answers again.
                self.error.set(None);
            }
            Err(error) => self.error.set(Some(format!("Failed to set the actuator: {error}"))),
        }
    }
}

/// Gives up on a request to `E` instead of waiting forever for a response that got lost.
async fn request<E: Endpoint>(response: impl Future<Output = Result<E::Response, HostErr<WireError>>>) -> Result<E::Response, String> {
    match select(pin!(response), TimeoutFuture::new(REQUEST_TIMEOUT_MS)).await {
        Either::Left((response, _)) => response.map_err(|error| format!("{error:?}")),
        Either::Right(((), _)) => Err(format!("No response on {:?} within {REQUEST_TIMEOUT_MS} ms", E::PATH)),
    }
}

#[component]
fn App() -> impl IntoView {
    let state = State::new();

    // Reconnect to a paired device right away if it is plugged in, otherwise as soon as it is.
    spawn_local(state.connect_paired());
    usb::on_connect(move |picolink| spawn_local(state.connect(picolink)));

    let connect = move |_| spawn_local(state.connect_picked());
    let status = move || match (state.error.get(), state.connection.with(Option::is_some), state.serial_number.get()) {
        (Some(error), _, _) => ("error", error),
        (None, false, _) => ("pending", "No device".to_owned()),
        (None, true, None) => ("pending", "Connecting…".to_owned()),
        (None, true, Some(_)) => ("ok", "Device connected".to_owned()),
    };

    view! {
        <header>
            <h1>"picolink"</h1>
            <span class=move || format!("pill {}", status().0)>{move || status().1}</span>
        </header>
        <main>
            <Show
                when=move || state.serial_number.get().is_some()
                fallback=move || {
                    view! {
                        <p class="hint">
                            <button on:click=connect disabled=move || state.connection.with(Option::is_some)>
                                "Connect…"
                            </button>
                        </p>
                    }
                }
            >
                <DeviceCard state />
                <ActuatorCard state />
                <SensorCard state />
            </Show>
        </main>
    }
}

#[component]
fn DeviceCard(state: State) -> impl IntoView {
    view! {
        <section class="card">
            <h2>"Device"</h2>
            <dl>
                <dt>"Connection"</dt>
                <dd>"WebUSB"</dd>
                <dt>"Serial number"</dt>
                <dd>{move || state.serial_number.get().map(|serial_number| format!("{serial_number:016x}"))}</dd>
            </dl>
            <button on:click=move |_| spawn_local(state.disconnect())>"Disconnect"</button>
        </section>
    }
}

#[component]
fn ActuatorCard(state: State) -> impl IntoView {
    view! {
        <section class="card">
            <h2>"Actuator"</h2>
            <p class="label">"Onboard LED"</p>
            <button
                class="toggle"
                class:on=move || state.actuator.get() == ActuatorState::On
                on:click=move |_| state.set_actuator(state.actuator.get_untracked().toggled())
            >
                {move || match state.actuator.get() {
                    ActuatorState::On => "On",
                    ActuatorState::Off => "Off",
                }}
            </button>
        </section>
    }
}

#[component]
fn SensorCard(state: State) -> impl IntoView {
    view! {
        <section class="card">
            <h2>"Sensor"</h2>
            <p class="label">"Chip temperature"</p>
            <p class="value">
                {move || state.reading.get().map_or("…".to_owned(), |reading| format!("{:.1} °C", reading.celsius))}
            </p>
        </section>
    }
}
