//! The USB device and the postcard-rpc server on top of it.
//!
//! The endpoints are up to the firmware: it defines them with [`postcard_rpc::define_dispatch!`],
//! with [`Tx`] as `tx_impl` and [`Spawn`] as `spawn_impl`, and passes the dispatcher to [`Server::new`].
//! Tasks can't be generic, so each firmware wraps [`Server::run`] in a task of its own.
//!
//! ```ignore
//! let app = RpcApp::new(context, spawner.into());
//! let (usb, server) = Server::new(usb::Driver::new(p.USB, Irqs), USB_PRODUCT, serial_number, app);
//! spawner.spawn(unwrap!(run_usb(usb)));
//! spawner.spawn(unwrap!(serve(server))); // `async fn serve(server: Server<RpcApp>) -> ! { server.run().await }`
//! ```

use defmt::debug;
use embassy_rp::{peripherals::USB, usb};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_usb::UsbDevice;
use picolink_protocol::{USB_MANUFACTURER, USB_PID, USB_VID};
use postcard_rpc::server::{
    Dispatch, Sender,
    impls::embassy_usb_v0_6::{
        PacketBuffers, USB_FS_MAX_PACKET_SIZE,
        dispatch_impl::{WireRxBuf, WireRxImpl, WireSpawnImpl, WireStorage, WireTxImpl},
    },
};
use static_cell::{ConstStaticCell, StaticCell};

pub type Driver = usb::Driver<'static, USB>;
pub type Usb = UsbDevice<'static, Driver>;
/// `tx_impl` for `define_dispatch!`.
pub type Tx = WireTxImpl<CriticalSectionRawMutex, Driver>;
/// `spawn_impl` for `define_dispatch!`.
pub type Spawn = WireSpawnImpl;
type Rx = WireRxImpl<Driver>;

/// The largest message the device can send, header included.
///
/// postcard-rpc's WebUSB client reads one 64-byte packet per transfer, so for WebUSB hosts, keep messages within 64 bytes.
pub const MAX_TX_MESSAGE: usize = 512;
/// The largest message the device can receive, header included. Hosts get no answer to longer ones.
pub const MAX_RX_MESSAGE: usize = 512;

/// How long the host may take to collect each USB packet the device sends, instead of postcard-rpc's 2 ms.
///
/// A send that times out leaves its frame unterminated, so the next send first writes an empty packet.
/// Every send after that then needs two IN transactions, and WebUSB only asks for the next one after a round trip
/// through JavaScript, which takes longer than 2 ms. Sends would keep timing out, and the device would never answer.
///
/// While no host is reading, every message the device publishes blocks the sender this long.
pub const SEND_TIMEOUT_MS_PER_PACKET: usize = 50;

static BUFFERS: ConstStaticCell<PacketBuffers<MAX_TX_MESSAGE, MAX_RX_MESSAGE>> = ConstStaticCell::new(PacketBuffers::new());
static STORAGE: WireStorage<CriticalSectionRawMutex, Driver> = WireStorage::new();
static SERIAL_STRING: StaticCell<[u8; 16]> = StaticCell::new();

/// Runs the USB device. Spawn it on the core that called [`Server::new`].
#[embassy_executor::task]
pub async fn run_usb(mut usb: Usb) -> ! {
    usb.run().await
}

/// A postcard-rpc server for the dispatcher `D`.
pub struct Server<D: Dispatch<Tx = Tx>> {
    inner: postcard_rpc::server::Server<Tx, Rx, WireRxBuf, D>,
    tx: Tx,
}

impl<D: Dispatch<Tx = Tx>> Server<D> {
    /// Sets up the USB device with a vendor-specific interface for postcard-rpc, and a server for `app`'s endpoints.
    /// Run the device with [`run_usb`] and the server with [`Server::run`]. Panics if called twice.
    ///
    /// `product` is the USB product string, the `USB_PRODUCT` of the firmware's module in `picolink_protocol`.
    /// `serial_number` becomes the USB serial number string, in hex, see [`crate::serial_number`].
    ///
    /// The driver enables the USB interrupt on the core that creates it, so create it on the core that runs [`run_usb`].
    pub fn new(driver: Driver, product: &'static str, serial_number: u64, app: D) -> (Usb, Self) {
        let mut config = embassy_usb::Config::new(USB_VID, USB_PID);
        config.manufacturer = Some(USB_MANUFACTURER);
        config.product = Some(product);
        // Tells devices apart, and browsers only remember WebUSB permissions for devices that have one.
        config.serial_number = Some(serial_string(serial_number));

        let buffers = BUFFERS.take();
        let (usb, tx, rx) = STORAGE.init(driver, config, buffers.tx_buf.as_mut_slice(), USB_FS_MAX_PACKET_SIZE);
        let key_kind = app.min_key_len();
        let inner = postcard_rpc::server::Server::new(tx.clone(), rx, buffers.rx_buf.as_mut_slice(), app, key_kind);
        (usb, Self { inner, tx })
    }

    /// For publishing topics to the host from other tasks.
    pub fn sender(&self) -> Sender<Tx> {
        self.inner.sender()
    }

    /// Serves one host after the other, forever.
    pub async fn run(mut self) -> ! {
        self.tx.set_timeout_ms_per_frame(SEND_TIMEOUT_MS_PER_PACKET).await;
        loop {
            // Returns when the host goes away.
            let _ = self.inner.run().await;
            debug!("Host disconnected");
        }
    }
}

/// `serial_number` in lowercase hex, as hosts expect it.
fn serial_string(serial_number: u64) -> &'static str {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let buffer = SERIAL_STRING.init([0; 16]);
    for (i, digit) in buffer.iter_mut().enumerate() {
        *digit = DIGITS[(serial_number >> (60 - 4 * i)) as usize & 0xF];
    }
    defmt::unwrap!(core::str::from_utf8(buffer))
}
