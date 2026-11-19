//! Finding a device through WebUSB and connecting a postcard-rpc client to it.

use js_sys::Reflect;
use picolink_protocol::{USB_PID, USB_VID};
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

/// A picolink device. Cheap to clone, all clones refer to the same device.
#[derive(Clone, PartialEq)]
pub struct Device(UsbDevice);

impl Device {
    /// Lets the user pick a device with the USB product string `product`. Only works from a user gesture, such as a click.
    ///
    /// WebUSB can't filter by product string, so the picker also offers the other picolink firmwares.
    pub async fn request(product: &str) -> Result<Self, String> {
        let filter = UsbDeviceFilter::new();
        filter.set_vendor_id(USB_VID);
        filter.set_product_id(USB_PID);
        filter.set_class_code(VENDOR_CLASS);
        let device = usb()?
            .request_device(&UsbDeviceRequestOptions::new(&[filter]))
            .await
            .map_err(js_error)?;
        let found = device.product_name();
        Self::from_device(device, product).ok_or_else(|| match found {
            Some(found) => format!("This page is for {product:?} devices, but this one is {found:?}: it runs other firmware"),
            None => format!("This page is for {product:?} devices, but this one has no USB product string"),
        })
    }

    /// A device with the USB product string `product` that the user granted access to before, if it is plugged in.
    pub async fn paired(product: &str) -> Result<Option<Self>, String> {
        let devices = usb()?.get_devices().await.map_err(js_error)?;
        Ok(devices.iter().find_map(|device| Self::from_device(device, product)))
    }

    fn from_device(device: UsbDevice, product: &str) -> Option<Self> {
        // Same check as the host library's.
        let matches = device.vendor_id() == USB_VID && device.product_id() == USB_PID && device.product_name().as_deref() == Some(product);
        matches.then_some(Self(device))
    }

    /// postcard-rpc's WebUSB client reads one packet per transfer, so every message the device sends must fit in 64 bytes.
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

/// Calls `f` whenever a device with the USB product string `product` that the user granted access to before gets plugged in.
///
/// WebUSB has one such listener per page, so call this once.
pub fn on_connect(product: &'static str, f: impl Fn(Device) + 'static) {
    let Ok(usb) = usb() else {
        return;
    };
    let listener = Closure::<dyn Fn(UsbConnectionEvent)>::new(move |event: UsbConnectionEvent| {
        if let Some(device) = Device::from_device(event.device(), product) {
            f(device);
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
