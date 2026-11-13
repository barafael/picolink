#![no_std]
#![no_main]
#![forbid(unsafe_code)]

//! picolink firmware for the RP2040.
//!
//! Core 0 runs the application: one [`sensor::Sensor`] and one [`actuator::Actuator`].
//! Core 1 runs the USB stack and the postcard-rpc server.
//! The cores only talk through the [`INCOMING`] and [`OUTGOING`] pub-sub channels.

use defmt::{info, unwrap, warn};
use defmt_rtt as _;
use embassy_executor::Executor;
use embassy_rp::{
    Peri,
    adc::{self, Adc},
    bind_interrupts,
    flash::{self, Flash},
    gpio::{Level, Output},
    multicore::{Stack, spawn_core1},
    peripherals::{FLASH, USB},
    usb,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, pubsub::PubSubChannel};
use panic_probe as _;
use static_cell::StaticCell;

use crate::{
    actuator::Actuator,
    constants::{CORE1_STACK_SIZE, FLASH_SIZE, INCOMING_CHANNEL_CAPACITY, OUTGOING_CHANNEL_CAPACITY},
    messages::{Command, Event},
    sensor::Sensor,
    tasks::{publish_readings, run_actuator, run_rpc_server, run_sensor, run_usb_device},
};

mod actuator;
mod constants;
mod messages;
mod rpc;
mod sensor;
mod tasks;

/// Core 0 to core 1. Published by the sensor and the actuator.
/// Subscribed by the reading publisher and, while it waits for an answer, by an RPC handler.
pub static OUTGOING: PubSubChannel<CriticalSectionRawMutex, Event, OUTGOING_CHANNEL_CAPACITY, 2, 2> = PubSubChannel::new();

/// Core 1 to core 0. Published by the RPC handlers, consumed by the actuator.
pub static INCOMING: PubSubChannel<CriticalSectionRawMutex, Command, INCOMING_CHANNEL_CAPACITY, 1, 1> = PubSubChannel::new();

static EXECUTOR0: StaticCell<Executor> = StaticCell::new();
static CORE1_STACK: StaticCell<Stack<CORE1_STACK_SIZE>> = StaticCell::new();
static EXECUTOR1: StaticCell<Executor> = StaticCell::new();

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
    ADC_IRQ_FIFO => adc::InterruptHandler;
});

#[cortex_m_rt::entry]
fn main() -> ! {
    info!("Hello picolink!");

    let p = embassy_rp::init(Default::default());

    // Must happen before core 1 runs: flash access pauses it.
    let serial_number = read_serial_number(p.FLASH);
    info!("serial number: {:016x}", serial_number);

    let actuator = Actuator::new(Output::new(p.PIN_25, Level::Low));
    let sensor = Sensor::new(
        Adc::new(p.ADC, Irqs, adc::Config::default()),
        adc::Channel::new_temp_sensor(p.ADC_TEMP_SENSOR),
    );
    let usb = p.USB;

    info!("Spawning tasks onto cores");

    spawn_core1(p.CORE1, CORE1_STACK.init(Stack::new()), move || {
        let commands = unwrap!(INCOMING.dyn_publisher());
        let readings = unwrap!(OUTGOING.dyn_subscriber());
        let executor1 = EXECUTOR1.init(Executor::new());
        executor1.run(|spawner| {
            // The driver enables the USB interrupt in the NVIC of the core that creates it, so this has to happen here.
            let (usb_device, server) = rpc::initialize(usb::Driver::new(usb, Irqs), serial_number, commands, spawner);
            let sender = server.sender();
            spawner.spawn(unwrap!(run_usb_device(usb_device)));
            spawner.spawn(unwrap!(run_rpc_server(server)));
            spawner.spawn(unwrap!(publish_readings(sender, readings)));
        })
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

fn read_serial_number(flash: Peri<'static, FLASH>) -> u64 {
    let mut flash = Flash::<_, flash::Blocking, FLASH_SIZE>::new_blocking(flash);
    let mut uid = [0; 8];
    if let Err(e) = flash.blocking_unique_id(&mut uid) {
        warn!("Failed to read flash unique id: {}", e);
    }
    u64::from_be_bytes(uid)
}
