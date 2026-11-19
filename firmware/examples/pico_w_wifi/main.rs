#![no_std]
#![no_main]
#![forbid(unsafe_code)]

//! A Pico W serving the `wifi` protocol: a continuously updated list of Wi-Fi networks, and joining one.
//!
//! Everything runs on core 0: the Wi-Fi chip's driver, the USB device, the postcard-rpc server and the scanner.
//! The scanner in [`wifi`] and the endpoints in [`rpc`] share the Wi-Fi chip through a mutex, so no channels are needed.

use cyw43::aligned_bytes;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts, dma,
    gpio::{Level, Output},
    peripherals::{DMA_CH0, PIO0, USB},
    pio::{self, Pio},
    usb,
};
use embassy_sync::mutex::Mutex;
use panic_probe as _;
use picolink_firmware::{
    link::{Link, Server, run_usb},
    serial_number,
};
use picolink_protocol::wifi::USB_PRODUCT;
use static_cell::StaticCell;

use crate::{
    rpc::RpcApp,
    wifi::{Wifi, scan_continuously},
};

mod rpc;
mod wifi;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>;
});

type WifiBus = cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello picolink Wi-Fi!");

    let p = embassy_rp::init(Default::default());
    let serial_number = serial_number::read(p.FLASH);
    info!("serial number: {:016x}", serial_number);

    // The Wi-Fi chip is wired to these pins on every Pico W. It runs its own firmware, loaded from here.
    let firmware = aligned_bytes!("../../cyw43-firmware/43439A0.bin");
    let clm = aligned_bytes!("../../cyw43-firmware/43439A0_clm.bin");
    let nvram = aligned_bytes!("../../cyw43-firmware/nvram_rp2040.bin");
    let power = Output::new(p.PIN_23, Level::Low);
    let chip_select = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        chip_select,
        p.PIN_24,
        p.PIN_29,
        dma::Channel::new(p.DMA_CH0, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    // No network stack: the network device is only needed for sending and receiving packets.
    let (_device, mut control, runner) = cyw43::new(STATE.init(cyw43::State::new()), power, spi, firmware, nvram).await;
    spawner.spawn(unwrap!(run_wifi(runner)));
    control.init(clm).await;
    control.set_power_management(cyw43::PowerManagementMode::PowerSave).await;
    info!("Wi-Fi chip ready");

    static WIFI: StaticCell<Wifi> = StaticCell::new();
    let wifi = WIFI.init(Mutex::new(control));

    let link = Link::new(usb::Driver::new(p.USB, Irqs), USB_PRODUCT, serial_number);
    let (usb_device, server) = link.serve(rpc::app(serial_number, wifi, spawner));
    let sender = server.sender();
    spawner.spawn(unwrap!(run_usb(usb_device)));
    spawner.spawn(unwrap!(serve(server)));
    spawner.spawn(unwrap!(scan_continuously(wifi, sender)));
}

#[embassy_executor::task]
async fn run_wifi(runner: cyw43::Runner<'static, WifiBus>) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn serve(server: Server<RpcApp>) -> ! {
    server.run().await
}
