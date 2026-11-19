//! The endpoints, served on core 1.
//!
//! The actuator endpoints are answered by core 0, via [`INCOMING`](crate::INCOMING) and [`OUTGOING`](crate::OUTGOING).

use defmt::{debug, unwrap};
use embassy_executor::Spawner;
use embassy_sync::pubsub::DynPublisher;
use picolink_firmware::link::{Spawn, Tx};
use picolink_protocol::sensor_actuator::{
    ActuatorState, ENDPOINT_LIST, GetActuatorEndpoint, SerialNumberEndpoint, SetActuatorEndpoint, TOPICS_IN_LIST, TOPICS_OUT_LIST,
};
use postcard_rpc::{define_dispatch, header::VarHeader};

use crate::{
    OUTGOING,
    messages::{Command, Event},
};

pub struct Context {
    serial_number: u64,
    commands: DynPublisher<'static, Command>,
}

define_dispatch! {
    app: RpcApp;
    spawn_fn: spawn_fn;
    tx_impl: Tx;
    spawn_impl: Spawn;
    context: Context;

    endpoints: {
        list: ENDPOINT_LIST;

        | EndpointTy            | kind      | handler               |
        | ----------            | ----      | -------               |
        | SerialNumberEndpoint  | blocking  | serial_number         |
        | GetActuatorEndpoint   | async     | get_actuator          |
        | SetActuatorEndpoint   | async     | set_actuator          |
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

pub fn app(serial_number: u64, commands: DynPublisher<'static, Command>, spawner: Spawner) -> RpcApp {
    RpcApp::new(Context { serial_number, commands }, spawner.into())
}

fn serial_number(context: &mut Context, _header: VarHeader, _request: ()) -> u64 {
    context.serial_number
}

async fn get_actuator(context: &mut Context, _header: VarHeader, _request: ()) -> ActuatorState {
    ask_actuator(context, Command::GetActuator).await
}

async fn set_actuator(context: &mut Context, _header: VarHeader, request: ActuatorState) -> ActuatorState {
    ask_actuator(context, Command::SetActuator(request)).await
}

/// Async handlers run one at a time, so the next actuator event is the answer.
async fn ask_actuator(context: &mut Context, command: Command) -> ActuatorState {
    // Subscribe before asking, so that the answer cannot be missed.
    let mut events = unwrap!(OUTGOING.dyn_subscriber());
    debug!("Asking actuator: {}", command);
    context.commands.publish(command).await;
    loop {
        if let Event::Actuator(state) = events.next_message_pure().await {
            return state;
        }
    }
}
