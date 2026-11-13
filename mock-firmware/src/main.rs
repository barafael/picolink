//! Runs the mock firmware as a USB device on this machine. See `usb_gadget` in the library.

use std::io::ErrorKind;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = picolink_mock_firmware::usb_gadget::run().await {
        eprintln!("Error: {error}");
        if error.kind() == ErrorKind::PermissionDenied {
            eprintln!("Creating a USB gadget needs root.");
        }
        std::process::exit(1);
    }
}
