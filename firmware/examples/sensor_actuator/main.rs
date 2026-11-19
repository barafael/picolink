#![no_std]
#![no_main]
#![forbid(unsafe_code)]

//! A plain Raspberry Pi Pico serving the `sensor_actuator` protocol: its temperature sensor and onboard LED.
//!
//! Core 0 runs the application: one [`sensor::Sensor`] and one [`actuator::Actuator`].
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
    link::{Link, run_usb},
    multicore::spawn_core1,
    serial_number,
    temperature::TemperatureSensor,
};
use picolink_protocol::sensor_actuator::USB_PRODUCT;
use static_cell::StaticCell;

use crate::{
    actuator::Actuator,
    messages::{Command, Event},
    sensor::Sensor,
    tasks::{publish_readings, run_actuator, run_sensor, serve},
};

mod actuator;
mod messages;
mod rpc;
mod sensor;
mod tasks;

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

    let actuator = Actuator::new(Output::new(p.PIN_25, Level::Low));
    let sensor = Sensor::new(TemperatureSensor::new(
        Adc::new(p.ADC, Irqs, adc::Config::default()),
        adc::Channel::new_temp_sensor(p.ADC_TEMP_SENSOR),
    ));
    let usb = p.USB;

    info!("Spawning tasks onto cores");

    spawn_core1(p.CORE1, move |spawner| {
        let commands = unwrap!(INCOMING.dyn_publisher());
        let readings = unwrap!(OUTGOING.dyn_subscriber());
        // Created here, so that core 1 handles the USB interrupt.
        let link = Link::new(usb::Driver::new(usb, Irqs), USB_PRODUCT, serial_number);
        let (usb_device, server) = link.serve(rpc::app(serial_number, commands, spawner));
        let sender = server.sender();
        spawner.spawn(unwrap!(run_usb(usb_device)));
        spawner.spawn(unwrap!(serve(server)));
        spawner.spawn(unwrap!(publish_readings(sender, readings)));
    });

    let sensor_publisher = unwrap!(OUTGOING.dyn_publisher());
    let actuator_publisher = unwrap!(OUTGOING.dyn_publisher());
    let actuator_subscriber = unwrap!(INCOMING.dyn_subscriber());
    let executor0 = EXECUTOR0.init(Executor::new());
    executor0.run(|spawner| {
        spawner.spawn(unwrap!(run_sensor(sensor, sensor_publisher)));
        spawner.spawn(unwrap!(run_actuator(actuator, actuator_subscriber, actuator_publisher)));
    });
}
