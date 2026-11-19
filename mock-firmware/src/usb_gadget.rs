//! The mock firmware as a USB device on this machine.
//!
//! `dummy_hcd` provides a virtual USB device controller that is wired to a virtual host controller.
//! A gadget bound to it shows up as an ordinary USB device, so the server's nusb path is exercised for real.
//! Needs root (configfs and FunctionFS) and `modprobe dummy_hcd`.

use std::io::{self, ErrorKind};

use picolink_protocol::{
    USB_MANUFACTURER, USB_PID, USB_VID,
    sensor_actuator::{
        ENDPOINT_LIST, GetActuatorEndpoint, SerialNumberEndpoint, SetActuatorEndpoint, TOPICS_IN_LIST, TOPICS_OUT_LIST, USB_PRODUCT,
    },
};
use postcard_rpc::server::{
    Dispatch, Server,
    impls::usb_gadget::dispatch_impl::{WireRxBuf, WireRxImpl, WireSpawnImpl, WireStorage, WireTxImpl},
};
use tokio::signal::unix::{SignalKind, signal};
use usb_gadget::{Class, Config, Gadget, Id, Strings, Udc};

use super::{Context, SERIAL_NUMBER, get_actuator, publish_readings, serial_number, set_actuator};

dispatcher!(GadgetMock, WireTxImpl, WireSpawnImpl);

/// Name of the gadget in configfs, so that only ours is ever removed.
const GADGET_NAME: &str = "picolink-mock";

/// Prefix of the virtual device controllers provided by `dummy_hcd`.
const DUMMY_UDC_PREFIX: &str = "dummy_udc";

/// Same as the firmware's packet buffers.
const BUFFER_SIZE: usize = 512;

static STORAGE: WireStorage = WireStorage::new();

/// Runs until SIGINT or SIGTERM, then removes the gadget again.
pub async fn run() -> io::Result<()> {
    let udc = dummy_udc()?;
    remove_stale_gadget()?;

    let mut gadget = Gadget::new(
        // Same as embassy-usb's defaults on the firmware: composite device with interface associations.
        Class::new(0xEF, 0x02, 0x01),
        Id::new(USB_VID, USB_PID),
        Strings::new(USB_MANUFACTURER, USB_PRODUCT, format!("{SERIAL_NUMBER:016x}")),
    );
    gadget.name = Some(GADGET_NAME.to_owned());

    let tx_buf = Box::leak(vec![0; BUFFER_SIZE].into_boxed_slice());
    let rx_buf = Box::leak(vec![0; BUFFER_SIZE].into_boxed_slice());
    let ((gadget, function), tx, rx) = STORAGE.init_without_build(gadget, tx_buf);
    let registration = gadget.with_config(Config::new("config").with_function(function)).bind(&udc)?;
    println!("Mock firmware is a USB device on {}", udc.name().to_string_lossy());

    let app = GadgetMock::new(Context::new(), tokio::runtime::Handle::current().into());
    let kkind = app.min_key_len();
    let server = Server::new(tx, rx, rx_buf, app, kkind);
    tokio::spawn(publish_readings(server.sender()));

    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        () = serve(server) => {}
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
    }

    println!("Removing the USB device");
    registration.remove()
}

/// Serves one host after the other, forever.
async fn serve(mut server: Server<WireTxImpl, WireRxImpl, WireRxBuf, GadgetMock>) {
    loop {
        // Returns when the host goes away.
        let _ = server.run().await;
    }
}

/// Left behind if an earlier run was killed.
fn remove_stale_gadget() -> io::Result<()> {
    for gadget in usb_gadget::registered()? {
        if gadget.name() == GADGET_NAME {
            println!("Removing gadget left over from an earlier run");
            gadget.remove()?;
        }
    }
    Ok(())
}

/// Never binds to a real device controller.
fn dummy_udc() -> io::Result<Udc> {
    usb_gadget::udcs()?
        .into_iter()
        .find(|udc| udc.name().to_string_lossy().starts_with(DUMMY_UDC_PREFIX))
        .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "no dummy USB device controller, run `sudo modprobe dummy_hcd`"))
}
