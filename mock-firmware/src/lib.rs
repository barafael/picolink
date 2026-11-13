//! Mock firmware: serves the picolink protocol like the real firmware, but without hardware.
//!
//! The same handlers run on one of two transports:
//! - [`in_process`]: in-process channels. Needs no setup, but bypasses USB entirely.
//! - [`usb_gadget`] (Linux, feature `usb-gadget`): a real USB device on this machine, through the kernel's
//!   `dummy_hcd`. The server finds and claims it exactly like the real firmware.

use std::time::Duration;

use picolink_protocol::{ActuatorState, ReadingTopic, SensorReading};
use postcard_rpc::{
    header::VarHeader,
    server::{Sender, WireTx},
};
use tokio::time::{Instant, MissedTickBehavior, interval};

/// Same as the firmware.
const SENSOR_INTERVAL: Duration = Duration::from_millis(500);
/// "MOCK" in ASCII.
pub const SERIAL_NUMBER: u64 = 0x4D4F_434B_0000_0001;

pub struct Context {
    actuator: ActuatorState,
}

impl Context {
    fn new() -> Self {
        Self {
            actuator: ActuatorState::Off,
        }
    }
}

/// Defines a dispatcher for the given transport. Each needs its own module, as `define_dispatch!` adds fixed-name items.
macro_rules! dispatcher {
    ($app:ident, $tx_impl:ty, $spawn_impl:ty) => {
        postcard_rpc::define_dispatch! {
            app: $app;
            spawn_fn: spawn_fn;
            tx_impl: $tx_impl;
            spawn_impl: $spawn_impl;
            context: Context;

            endpoints: {
                list: ENDPOINT_LIST;

                | EndpointTy            | kind      | handler               |
                | ----------            | ----      | -------               |
                | SerialNumberEndpoint  | blocking  | serial_number         |
                | GetActuatorEndpoint   | blocking  | get_actuator          |
                | SetActuatorEndpoint   | blocking  | set_actuator          |
            };
            topics_in: {
                list: TOPICS_IN_LIST;

                | TopicTy               | kind      | handler               |
                | -------               | ----      | -------               |
            };
            topics_out: {
                list: TOPICS_OUT_LIST;
            };
        }
    };
}

mod channels {
    use picolink_protocol::{
        ENDPOINT_LIST, GetActuatorEndpoint, SerialNumberEndpoint, SetActuatorEndpoint, TOPICS_IN_LIST, TOPICS_OUT_LIST,
    };
    use postcard_rpc::{
        header::VarSeqKind,
        host_client::{HostClient, test_channels::new_from_channels},
        server::{
            Dispatch, Server,
            impls::test_channels::{
                ChannelWireRx, ChannelWireSpawn, ChannelWireTx,
                dispatch_impl::{Settings, WireRxBuf, WireRxImpl, WireSpawnImpl, WireTxImpl, new_server},
            },
        },
        standard_icd::WireError,
    };

    use super::{Context, get_actuator, publish_readings, serial_number, set_actuator};

    dispatcher!(ChannelMock, WireTxImpl, WireSpawnImpl);

    /// The mock firmware, and a client connected to it over in-process channels.
    ///
    /// Running the mock firmware's future, for example by spawning it, is up to the caller.
    /// It completes once the client is closed.
    pub fn in_process() -> (HostClient<WireError>, impl Future<Output = ()>) {
        let (client_tx, server_rx) = tokio::sync::mpsc::channel(16);
        let (server_tx, client_rx) = tokio::sync::mpsc::channel(16);

        let app = ChannelMock::new(Context::new(), ChannelWireSpawn {});
        let kkind = app.min_key_len();
        let settings = Settings {
            tx: ChannelWireTx::new(server_tx),
            rx: ChannelWireRx::new(server_rx),
            buf: 1024,
            kkind,
        };
        let client = new_from_channels(client_tx, client_rx, VarSeqKind::Seq4);
        (client, serve(new_server(app, settings)))
    }

    async fn serve(mut server: Server<WireTxImpl, WireRxImpl, WireRxBuf, ChannelMock>) {
        let sender = server.sender();
        tokio::select! {
            // Returns once the client is gone.
            _ = server.run() => {}
            _ = publish_readings(sender) => {}
        }
    }
}

pub use channels::in_process;

#[cfg(all(target_os = "linux", feature = "usb-gadget"))]
pub mod usb_gadget;

fn serial_number(_context: &mut Context, _header: VarHeader, _request: ()) -> u64 {
    SERIAL_NUMBER
}

fn get_actuator(context: &mut Context, _header: VarHeader, _request: ()) -> ActuatorState {
    context.actuator
}

fn set_actuator(context: &mut Context, _header: VarHeader, request: ActuatorState) -> ActuatorState {
    context.actuator = request;
    context.actuator
}

/// A slowly swinging temperature around 25 °C. Like the firmware, readings nobody receives are dropped.
async fn publish_readings<Tx: WireTx + Clone>(sender: Sender<Tx>) {
    let start = Instant::now();
    let mut ticker = interval(SENSOR_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut seq = 0u32;
    loop {
        ticker.tick().await;
        let uptime = start.elapsed();
        let reading = SensorReading {
            uptime_ms: uptime.as_millis() as u64,
            celsius: 25.0 + 2.0 * (uptime.as_secs_f32() / 10.0).sin(),
        };
        let _ = sender.publish::<ReadingTopic>(seq.into(), &reading).await;
        seq = seq.wrapping_add(1);
    }
}

#[cfg(test)]
mod test {
    use picolink_protocol::{GetActuatorEndpoint, SerialNumberEndpoint, SetActuatorEndpoint};

    use super::*;

    #[tokio::test]
    async fn serves_the_protocol() {
        let (client, firmware) = in_process();
        let firmware = tokio::spawn(firmware);
        let mut readings = client.subscribe_multi::<ReadingTopic>(8).await.unwrap();

        assert_eq!(client.send_resp::<SerialNumberEndpoint>(&()).await.unwrap(), SERIAL_NUMBER);
        assert_eq!(client.send_resp::<GetActuatorEndpoint>(&()).await.unwrap(), ActuatorState::Off);
        assert_eq!(
            client.send_resp::<SetActuatorEndpoint>(&ActuatorState::On).await.unwrap(),
            ActuatorState::On
        );
        assert_eq!(client.send_resp::<GetActuatorEndpoint>(&()).await.unwrap(), ActuatorState::On);

        let first = readings.recv().await.unwrap();
        let second = readings.recv().await.unwrap();
        assert!(second.uptime_ms > first.uptime_ms);

        client.close();
        firmware.await.unwrap();
    }
}
