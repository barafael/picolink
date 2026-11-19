//! The `sensor_actuator` firmware's page: the same UI as `app/`, without a server.

use std::convert::Infallible;

use leptos::{prelude::*, task::spawn_local};
use picolink_protocol::sensor_actuator::{
    ActuatorState, GetActuatorEndpoint, ReadingTopic, SensorReading, SerialNumberEndpoint, SetActuatorEndpoint, USB_PRODUCT,
};
use picolink_webusb::{Client, Connection, DEFAULT_TIMEOUT_MS, Page, request};
use postcard_rpc::host_client::MultiSubRxError;

const READING_DEPTH: usize = 16;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[derive(Clone, Copy)]
struct State {
    connection: Connection,
    actuator: RwSignal<ActuatorState>,
    reading: RwSignal<Option<SensorReading>>,
}

impl State {
    fn new() -> Self {
        Self {
            connection: Connection::new(USB_PRODUCT),
            actuator: RwSignal::new(ActuatorState::Off),
            reading: RwSignal::new(None),
        }
    }

    /// Runs for as long as the device is connected.
    async fn session(self, client: Client) -> Result<Infallible, String> {
        // Left over from an earlier connection.
        self.reading.set(None);
        // Protocol version mismatches show up here: the device rejects requests it doesn't know.
        let serial_number = request::<SerialNumberEndpoint>(&client, &(), DEFAULT_TIMEOUT_MS).await?;
        let actuator = request::<GetActuatorEndpoint>(&client, &(), DEFAULT_TIMEOUT_MS).await?;
        let mut readings = client
            .subscribe_multi::<ReadingTopic>(READING_DEPTH)
            .await
            .map_err(|_| "Client closed")?;
        self.actuator.set(actuator);
        self.connection.identified(serial_number);
        loop {
            match readings.recv().await {
                Ok(reading) => self.reading.set(Some(reading)),
                Err(MultiSubRxError::Lagged(_)) => {}
                Err(MultiSubRxError::IoClosed) => return Err("Device disconnected".into()),
            }
        }
    }

    fn set_actuator(self, state: ActuatorState) {
        spawn_local(async move {
            match self.connection.request::<SetActuatorEndpoint>(&state, DEFAULT_TIMEOUT_MS).await {
                Some(Ok(state)) => {
                    self.actuator.set(state);
                    // An earlier request may have failed, but the device answers again.
                    self.connection.set_error(None);
                }
                Some(Err(error)) => self.connection.set_error(Some(format!("Failed to set the actuator: {error}"))),
                None => {}
            }
        });
    }
}

#[component]
fn App() -> impl IntoView {
    let state = State::new();
    view! {
        <Page title="picolink" connection=state.connection session=move |client| state.session(client)>
            <DeviceCard state />
            <ActuatorCard state />
            <SensorCard state />
        </Page>
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
                <dd>{move || state.connection.serial_number().map(|serial_number| format!("{serial_number:016x}"))}</dd>
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
