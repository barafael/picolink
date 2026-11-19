//! The `pico_w_wifi` firmware's page: the Wi-Fi networks a Pico W sees, updated with every scan, and joining one.

use std::convert::Infallible;

use leptos::{prelude::*, task::spawn_local};
use picolink_protocol::wifi::{
    JOIN_ANSWER_TIMEOUT_MS, JoinEndpoint, JoinError, JoinRequest, LEAVE_ANSWER_TIMEOUT_MS, LeaveEndpoint, MAX_NETWORKS, Network,
    PASSPHRASE_LEN, Passphrase, ScanEvent, ScanTopic, SerialNumberEndpoint, Ssid, StatusEndpoint, USB_PRODUCT,
};
use picolink_webusb::{Client, Connection, DEFAULT_TIMEOUT_MS, request};
use postcard_rpc::host_client::MultiSubRxError;

/// A whole scan, and then some.
const EVENT_DEPTH: usize = 2 * (MAX_NETWORKS + 1);

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[derive(Clone, Copy)]
struct State {
    connection: Connection,
    /// `Some` once the device identified itself.
    serial_number: RwSignal<Option<u64>>,
    joined: RwSignal<Option<String>>,
    /// From the last complete scan, strongest first.
    networks: RwSignal<Vec<Network>>,
    /// Complete scans received in this session.
    scans: RwSignal<u32>,
    form: JoinForm,
}

/// What the user typed into the join form, and how joining went.
#[derive(Clone, Copy)]
struct JoinForm {
    ssid: RwSignal<String>,
    passphrase: RwSignal<String>,
    busy: RwSignal<bool>,
    /// `Ok` for success, `Err` for failure.
    outcome: RwSignal<Option<Result<String, String>>>,
}

impl State {
    fn new() -> Self {
        Self {
            connection: Connection::new(USB_PRODUCT),
            serial_number: RwSignal::new(None),
            joined: RwSignal::new(None),
            networks: RwSignal::new(Vec::new()),
            scans: RwSignal::new(0),
            form: JoinForm {
                ssid: RwSignal::new(String::new()),
                passphrase: RwSignal::new(String::new()),
                busy: RwSignal::new(false),
                outcome: RwSignal::new(None),
            },
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
        self.networks.set(Vec::new());
        self.scans.set(0);
        self.form.busy.set(false);
        self.form.outcome.set(None);

        // Before anything else, so that no scan event is missed.
        let mut events = client
            .subscribe_multi::<ScanTopic>(EVENT_DEPTH)
            .await
            .map_err(|_| "Client closed")?;
        // Protocol version mismatches show up here: the device rejects requests it doesn't know.
        let serial_number = request::<SerialNumberEndpoint>(&client, &(), DEFAULT_TIMEOUT_MS).await?;
        let status = request::<StatusEndpoint>(&client, &(), DEFAULT_TIMEOUT_MS).await?;
        self.joined.set(status.joined.map(|ssid| ssid.as_str().to_owned()));
        self.serial_number.set(Some(serial_number));

        // The scan being received, and its networks so far.
        let mut pending: Option<(u16, Vec<Network>)> = None;
        loop {
            match events.recv().await {
                Ok(ScanEvent::Network { scan, network }) => match &mut pending {
                    Some((current, networks)) if *current == scan => networks.push(network),
                    // A new scan. What came before belonged to one whose end got lost.
                    _ => pending = Some((scan, vec![network])),
                },
                Ok(ScanEvent::Done { scan, networks: sent }) => {
                    let received = match pending.take() {
                        Some((current, networks)) if current == scan => networks,
                        _ => Vec::new(),
                    };
                    // Only complete scans: a partial one is usually the end of the scan that was being sent when
                    // the page connected, so the weakest networks only. The next scan follows within seconds.
                    if received.len() == usize::from(sent) {
                        self.networks.set(received);
                        self.scans.update(|scans| *scans += 1);
                    }
                }
                Err(MultiSubRxError::Lagged(_)) => pending = None,
                Err(MultiSubRxError::IoClosed) => return Err("Device disconnected".into()),
            }
        }
    }

    fn join(self) {
        let ssid = self.form.ssid.get_untracked();
        let join = match join_request(&ssid, &self.form.passphrase.get_untracked()) {
            Ok(join) => join,
            Err(error) => {
                self.form.outcome.set(Some(Err(error)));
                return;
            }
        };
        let Some((device, client)) = self.connection.current() else {
            return;
        };
        self.form.busy.set(true);
        self.form.outcome.set(None);
        spawn_local(async move {
            let result = request::<JoinEndpoint>(&client, &join, JOIN_ANSWER_TIMEOUT_MS as u32).await;
            self.form.busy.set(false);
            match result {
                Ok(Ok(())) => {
                    self.joined.set(Some(ssid.clone()));
                    self.form.passphrase.set(String::new());
                    self.form.outcome.set(Some(Ok(format!("Joined {ssid:?}"))));
                }
                Ok(Err(error)) => {
                    // The device dropped the network it was on, too.
                    self.joined.set(None);
                    self.form
                        .outcome
                        .set(Some(Err(format!("Joining {ssid:?} failed: {}", describe(error)))));
                }
                // After a deliberate disconnect, the request fails because the client was closed.
                Err(error) if self.connection.is_current(&device) => self.form.outcome.set(Some(Err(error))),
                Err(_) => {}
            }
        });
    }

    fn leave(self) {
        let Some((device, client)) = self.connection.current() else {
            return;
        };
        spawn_local(async move {
            match request::<LeaveEndpoint>(&client, &(), LEAVE_ANSWER_TIMEOUT_MS as u32).await {
                Ok(()) => {
                    self.joined.set(None);
                    self.form.outcome.set(None);
                }
                Err(error) if self.connection.is_current(&device) => self.form.outcome.set(Some(Err(format!("Leaving failed: {error}")))),
                Err(_) => {}
            }
        });
    }
}

/// An empty passphrase is for open networks.
fn join_request(ssid: &str, passphrase: &str) -> Result<JoinRequest, String> {
    if ssid.is_empty() {
        return Err("Pick a network, or type the name of a hidden one".into());
    }
    let ssid = Ssid::try_from(ssid).map_err(|_| format!("Network names have at most {} bytes", Ssid::new().capacity()))?;
    let passphrase = if passphrase.is_empty() {
        None
    } else if PASSPHRASE_LEN.contains(&passphrase.len()) {
        Some(Passphrase::try_from(passphrase).expect("length checked"))
    } else {
        return Err(format!(
            "Passphrases have {} to {} characters",
            PASSPHRASE_LEN.start(),
            PASSPHRASE_LEN.end()
        ));
    };
    Ok(JoinRequest { ssid, passphrase })
}

fn describe(error: JoinError) -> String {
    match error {
        JoinError::NetworkNotFound => "network not found".into(),
        JoinError::AuthenticationFailed => "authentication failed, check the passphrase".into(),
        JoinError::Failed(status) => format!("the Wi-Fi chip reported status {status}"),
        JoinError::Timeout => "no answer from the network in time".into(),
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
            <h1>"picolink Wi-Fi"</h1>
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
                <JoinCard state />
                <NetworksCard state />
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
                <dt>"Network"</dt>
                <dd class="joined">{move || state.joined.get().unwrap_or_else(|| "none".into())}</dd>
            </dl>
            <p class="actions">
                <Show when=move || state.joined.get().is_some()>
                    <button on:click=move |_| state.leave()>"Leave network"</button>
                </Show>
                <button on:click=move |_| spawn_local(state.connection.disconnect())>"Disconnect"</button>
            </p>
        </section>
    }
}

#[component]
fn JoinCard(state: State) -> impl IntoView {
    let form = state.form;
    view! {
        <section class="card">
            <h2>"Join"</h2>
            <form class="join" on:submit=move |event| {
                event.prevent_default();
                state.join();
            }>
                <label>
                    "Network"
                    <input type="text" name="ssid" placeholder="Pick one below, or type a hidden one" bind:value=form.ssid />
                </label>
                <label>
                    "Passphrase"
                    <input
                        type="password"
                        name="passphrase"
                        autocomplete="off"
                        placeholder="Empty for open networks"
                        bind:value=form.passphrase
                    />
                </label>
                <button type="submit" disabled=move || form.busy.get()>
                    {move || if form.busy.get() { "Joining…" } else { "Join" }}
                </button>
            </form>
            {move || {
                form.outcome
                    .get()
                    .map(|outcome| match outcome {
                        Ok(message) => view! { <p class="outcome ok">{message}</p> },
                        Err(message) => view! { <p class="outcome error">{message}</p> },
                    })
            }}
        </section>
    }
}

#[component]
fn NetworksCard(state: State) -> impl IntoView {
    let pick = move |network: &Network| {
        state.form.ssid.set(network.ssid.as_str().to_owned());
        state.form.passphrase.set(String::new());
        state.form.outcome.set(None);
    };
    view! {
        <section class="card wide">
            <h2>
                "Networks "
                <span class="scan">
                    {move || match state.scans.get() {
                        0 => "scanning…".to_owned(),
                        scans => format!("{} found, scan {scans}", state.networks.with(Vec::len)),
                    }}
                </span>
            </h2>
            <table class="networks">
                <thead>
                    <tr>
                        <th>"Name"</th>
                        <th>"Signal"</th>
                        <th>"Channel"</th>
                        <th>"Security"</th>
                    </tr>
                </thead>
                <tbody>
                    // Keyed by everything a row shows: rows are rendered once per key, not updated.
                    <For each=move || state.networks.get() key=|network| network.clone() let:network>
                        {
                            let joined = {
                                let ssid = network.ssid.clone();
                                move || state.joined.with(|joined| joined.as_deref() == Some(ssid.as_str()))
                            };
                            let name = if network.ssid.is_empty() { "(hidden)".to_owned() } else { network.ssid.as_str().to_owned() };
                            let security = if network.secured { "secured" } else { "open" };
                            let (rssi, channel) = (network.rssi, network.channel);
                            view! {
                                <tr class:joined=joined on:click=move |_| pick(&network)>
                                    <td>{name}</td>
                                    <td>
                                        <meter min="-100" max="-30" low="-80" high="-60" optimum="-30" value=rssi.to_string()></meter>
                                        " "
                                        {format!("{rssi} dBm")}
                                    </td>
                                    <td>{channel}</td>
                                    <td>{security}</td>
                                </tr>
                            }
                        }
                    </For>
                </tbody>
            </table>
        </section>
    }
}
