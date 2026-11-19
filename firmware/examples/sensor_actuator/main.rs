#![no_std]
#![no_main]
#![forbid(unsafe_code)]

//! A plain Raspberry Pi Pico serving the `sensor_actuator` protocol: its temperature sensor and onboard LED.
//!
//! Core 0 runs the application: [`sensor::run`] and [`actuator::run`].
//! Core 1 runs the USB stack and the postcard-rpc server.
//! The cores only talk through the [`INCOMING`] and [`OUTGOING`] pub-sub channels.
//!
//! On a Pico W, the onboard LED is wired to the Wi-Fi chip instead, so it stays dark.

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Executor;
use embassy_rp::{
    adc::{self, Adc},
    bind_interrupts,
    gpio::{Level, Output},
    peripherals::USB,
    usb,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pubsub::PubSubChannel};
use panic_probe as _;
use picolink_firmware::{
    link::{Server, run_usb},
    multicore::spawn_core1,
    serial_number,
    temperature::TemperatureSensor,
};
use picolink_protocol::sensor_actuator::USB_PRODUCT;
use static_cell::StaticCell;

use crate::messages::{Command, Event};

mod actuator;
mod messages;
mod rpc;
mod sensor;

const OUTGOING_CHANNEL_CAPACITY: usize = 16;
const INCOMING_CHANNEL_CAPACITY: usize = 8;

/// Core 0 to core 1. Published by the sensor and the actuator.
/// Subscribed by the reading publisher and, while it waits for an answer, by an RPC handler.
pub static OUTGOING: PubSubChannel<CriticalSectionRawMutex, Event, OUTGOING_CHANNEL_CAPACITY, 2, 2> = PubSubChannel::new();

/// Core 1 to core 0. Published by the RPC handlers, consumed by the actuator.
pub static INCOMING: PubSubChannel<CriticalSectionRawMutex, Command, INCOMING_CHANNEL_CAPACITY, 1, 1> = PubSubChannel::new();

static EXECUTOR0: StaticCell<Executor> = StaticCell::new();

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
});

#[cortex_m_rt::entry]
fn main() -> ! {
    info!("Hello picolink!");

    let p = embassy_rp::init(Default::default());

    // Must happen before core 1 runs.
    let serial_number = serial_number::read(p.FLASH);
    info!("serial number: {:016x}", serial_number);

    let led = Output::new(p.PIN_25, Level::Low);
    let temperature = TemperatureSensor::new(
        Adc::new(p.ADC, Irqs, adc::Config::default()),
        adc::Channel::new_temp_sensor(p.ADC_TEMP_SENSOR),
    );
    let usb = p.USB;

    info!("Spawning tasks onto cores");

    spawn_core1(p.CORE1, move |spawner| {
        let commands = unwrap!(INCOMING.dyn_publisher());
        let readings = unwrap!(OUTGOING.dyn_subscriber());
        // Created here, so that core 1 handles the USB interrupt.
        let app = rpc::app(serial_number, commands, spawner);
        let (usb_device, server) = Server::new(usb::Driver::new(usb, Irqs), USB_PRODUCT, serial_number, app);
        let sender = server.sender();
        spawner.spawn(unwrap!(run_usb(usb_device)));
        spawner.spawn(unwrap!(rpc::serve(server)));
        spawner.spawn(unwrap!(rpc::publish_readings(sender, readings)));
    });

    let sensor_publisher = unwrap!(OUTGOING.dyn_publisher());
    let actuator_publisher = unwrap!(OUTGOING.dyn_publisher());
    let actuator_subscriber = unwrap!(INCOMING.dyn_subscriber());
    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| {
        spawner.spawn(unwrap!(sensor::run(temperature, sensor_publisher)));
        spawner.spawn(unwrap!(actuator::run(led, actuator_subscriber, actuator_publisher)));
    });
}
