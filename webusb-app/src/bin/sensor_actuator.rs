//! The `sensor_actuator` firmware's page: the same UI as `app/`, without a server.

use std::convert::Infallible;

use leptos::{prelude::*, task::spawn_local};
use picolink_protocol::sensor_actuator::{
    ActuatorState, GetActuatorEndpoint, ReadingTopic, SensorReading, SerialNumberEndpoint, SetActuatorEndpoint, USB_PRODUCT,
};
use picolink_webusb::{Client, Connection, DEFAULT_TIMEOUT_MS, request, usb::Device};
use postcard_rpc::host_client::MultiSubRxError;

const READING_DEPTH: usize = 16;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[derive(Clone, Copy)]
struct State {
    connection: Connection,
    /// `Some` once the device identified itself.
    serial_number: RwSignal<Option<u64>>,
    actuator: RwSignal<ActuatorState>,
    reading: RwSignal<Option<SensorReading>>,
}

impl State {
    fn new() -> Self {
        Self {
            connection: Connection::new(USB_PRODUCT),
            serial_number: RwSignal::new(None),
            actuator: RwSignal::new(ActuatorState::Off),
            reading: RwSignal::new(None),
        }
    }

    /// Identified once connected and the device told its serial number.
    fn identified(self) -> Option<u64> {
        self.serial_number.get().filter(|_| self.connection.is_connected())
    }

    /// Runs for as long as the device is connected.
    async fn session(self, client: Client) -> Result<Infallible, String> {
        // Left over from an earlier connection.
        self.serial_number.set(None);
        self.reading.set(None);
        // Protocol version mismatches show up here: the device rejects requests it doesn't know.
        let serial_number = request::<SerialNumberEndpoint>(&client, &(), DEFAULT_TIMEOUT_MS).await?;
        let actuator = request::<GetActuatorEndpoint>(&client, &(), DEFAULT_TIMEOUT_MS).await?;
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
        let Some((device, client)) = self.connection.current() else {
            return;
        };
        spawn_local(self.send_actuator(device, client, state));
    }

    async fn send_actuator(self, device: Device, client: Client, state: ActuatorState) {
        match request::<SetActuatorEndpoint>(&client, &state, DEFAULT_TIMEOUT_MS).await {
            Ok(state) => {
                self.actuator.set(state);
                // An earlier request may have failed, but the device answers again.
                self.connection.set_error(None);
            }
            // After a deliberate disconnect, the request fails because the client was closed.
            Err(error) if self.connection.is_current(&device) => {
                self.connection.set_error(Some(format!("Failed to set the actuator: {error}")))
            }
            Err(_) => {}
        }
    }
}

#[component]
fn App() -> impl IntoView {
    let state = State::new();
    state.connection.connect_automatically(move |client| state.session(client));

    let connect = move |_| spawn_local(state.connection.connect_picked(move |client| state.session(client)));
    let status = move || match (state.connection.error(), state.connection.is_connected(), state.identified()) {
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
                when=move || state.identified().is_some()
                fallback=move || {
                    view! {
                        <p class="hint">
                            <button on:click=connect disabled=move || state.connection.is_connected()>
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
            <button on:click=move |_| spawn_local(state.connection.disconnect())>"Disconnect"</button>
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
