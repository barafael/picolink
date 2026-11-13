//! Finding the device through WebUSB and connecting a postcard-rpc client to it.

use js_sys::Reflect;
use picolink_protocol::{USB_PID, USB_PRODUCT, USB_VID};
use postcard_rpc::{
    header::VarSeqKind,
    host_client::HostClient,
    standard_icd::{ERROR_PATH, WireError},
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{Usb, UsbConnectionEvent, UsbDevice, UsbDeviceFilter, UsbDeviceRequestOptions};

pub type Client = HostClient<WireError>;

/// Outgoing request queue.
const CLIENT_DEPTH: usize = 8;
/// postcard-rpc's interface. The MT21 shares VID and PID, but has none.
const VENDOR_CLASS: u8 = 0xFF;

/// Cheap to clone, all clones refer to the same device.
#[derive(Clone, PartialEq)]
pub struct Picolink(UsbDevice);

impl Picolink {
    /// Lets the user pick a device. Only works from a user gesture, such as a click.
    pub async fn request() -> Result<Self, String> {
        let filter = UsbDeviceFilter::new();
        filter.set_vendor_id(USB_VID);
        filter.set_product_id(USB_PID);
        filter.set_class_code(VENDOR_CLASS);
        let device = usb()?
            .request_device(&UsbDeviceRequestOptions::new(&[filter]))
            .await
            .map_err(js_error)?;
        Self::from_device(device).ok_or_else(|| "Not a picolink device".into())
    }

    /// A device the user already granted access to on an earlier visit, if it is plugged in.
    pub async fn paired() -> Result<Option<Self>, String> {
        let devices = usb()?.get_devices().await.map_err(js_error)?;
        Ok(devices.iter().find_map(Self::from_device))
    }

    fn from_device(device: UsbDevice) -> Option<Self> {
        // Same check as the server's.
        let is_picolink =
            device.vendor_id() == USB_VID && device.product_id() == USB_PID && device.product_name().as_deref() == Some(USB_PRODUCT);
        is_picolink.then_some(Self(device))
    }

    /// postcard-rpc's WebUSB client reads one packet per transfer, so every message must stay below 64 bytes.
    pub async fn connect(&self) -> Result<Client, String> {
        self.0.open().await.map_err(js_error)?;
        // The client needs a configuration to find the vendor-specific interface in.
        if self.0.configuration().is_none() {
            self.0.select_configuration(1).await.map_err(js_error)?;
        }
        HostClient::try_new_webusb_from_device(self.0.clone(), ERROR_PATH, CLIENT_DEPTH, VarSeqKind::Seq4)
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn close(&self) {
        // Fails if the device is already gone, which is just as good.
        let _ = self.0.close().await;
    }
}

/// Calls `f` whenever a device the user granted access to before gets plugged in.
pub fn on_connect(f: impl Fn(Picolink) + 'static) {
    let Ok(usb) = usb() else {
        return;
    };
    let listener = Closure::<dyn Fn(UsbConnectionEvent)>::new(move |event: UsbConnectionEvent| {
        if let Some(picolink) = Picolink::from_device(event.device()) {
            f(picolink);
        }
    });
    usb.set_onconnect(Some(listener.as_ref().unchecked_ref()));
    // Lives as long as the page.
    listener.forget();
}

fn usb() -> Result<Usb, String> {
    let navigator = web_sys::window().ok_or("No window")?.navigator();
    // `navigator.usb` is missing outside of Chromium and on non-secure origins (anything but https and localhost).
    if !Reflect::has(&navigator, &"usb".into()).unwrap_or(false) {
        return Err("WebUSB is not available. Use a Chromium-based browser and serve the page from localhost or https.".into());
    }
    Ok(navigator.usb())
}

fn js_error(error: JsValue) -> String {
    match error.dyn_ref::<js_sys::Error>() {
        Some(error) => error.message().into(),
        None => format!("{error:?}"),
    }
}
