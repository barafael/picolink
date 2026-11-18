//! The postcard-rpc server on core 1.
//!
//! Endpoints that concern the application are answered by core 0, via [`INCOMING`](crate::INCOMING) and
//! [`OUTGOING`](crate::OUTGOING).

use defmt::{debug, unwrap};
use embassy_executor::Spawner;
use embassy_rp::{peripherals::USB, usb};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pubsub::DynPublisher};
use embassy_usb::UsbDevice;
use picolink_protocol::{
    ActuatorState, ENDPOINT_LIST, GetActuatorEndpoint, SerialNumberEndpoint, SetActuatorEndpoint, TOPICS_IN_LIST, TOPICS_OUT_LIST,
    USB_MANUFACTURER, USB_PID, USB_PRODUCT, USB_VID,
};
use postcard_rpc::{
    define_dispatch,
    header::VarHeader,
    server::{
        Dispatch, Server,
        impls::embassy_usb_v0_6::{
            PacketBuffers, USB_FS_MAX_PACKET_SIZE,
            dispatch_impl::{WireRxBuf, WireRxImpl, WireSpawnImpl, WireStorage, WireTxImpl},
        },
    },
};
use static_cell::{ConstStaticCell, StaticCell};

use crate::{
    OUTGOING,
    messages::{Command, Event},
};

type AppDriver = usb::Driver<'static, USB>;
pub type AppTx = WireTxImpl<CriticalSectionRawMutex, AppDriver>;
type AppRx = WireRxImpl<AppDriver>;
pub type AppServer = Server<AppTx, AppRx, WireRxBuf, RpcApp>;

static PACKET_BUFFERS: ConstStaticCell<PacketBuffers<512, 512>> = ConstStaticCell::new(PacketBuffers::new());
static STORAGE: WireStorage<CriticalSectionRawMutex, AppDriver> = WireStorage::new();
static SERIAL_STRING: StaticCell<[u8; 16]> = StaticCell::new();

pub struct Context {
    serial_number: u64,
    commands: DynPublisher<'static, Command>,
}

define_dispatch! {
    app: RpcApp;
    spawn_fn: spawn_fn;
    tx_impl: AppTx;
    spawn_impl: WireSpawnImpl;
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

/// Must be called on the core that runs the returned device and server.
pub fn initialize(
    driver: AppDriver,
    serial_number: u64,
    commands: DynPublisher<'static, Command>,
    spawner: Spawner,
) -> (UsbDevice<'static, AppDriver>, AppServer) {
    let mut config = embassy_usb::Config::new(USB_VID, USB_PID);
    config.manufacturer = Some(USB_MANUFACTURER);
    config.product = Some(USB_PRODUCT);
    // Tells devices apart, and browsers only remember WebUSB permissions for devices that have one.
    config.serial_number = Some(serial_string(serial_number));

    let buffers = PACKET_BUFFERS.take();
    let (device, tx, rx) = STORAGE.init(driver, config, buffers.tx_buf.as_mut_slice(), USB_FS_MAX_PACKET_SIZE);

    let app = RpcApp::new(Context { serial_number, commands }, spawner.into());
    let key_kind = app.min_key_len();
    let server = Server::new(tx, rx, buffers.rx_buf.as_mut_slice(), app, key_kind);
    (device, server)
}

/// The USB serial number string: `serial_number` in lowercase hex, as the mock firmware formats it.
fn serial_string(serial_number: u64) -> &'static str {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let buffer = SERIAL_STRING.init([0; 16]);
    for (i, digit) in buffer.iter_mut().enumerate() {
        *digit = DIGITS[(serial_number >> (60 - 4 * i)) as usize & 0xF];
    }
    unwrap!(core::str::from_utf8(buffer))
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
