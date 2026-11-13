use defmt::{debug, trace};
use embassy_rp::{peripherals::USB, usb};
use embassy_sync::pubsub::{DynPublisher, DynSubscriber};
use embassy_usb::UsbDevice;
use picolink_protocol::ReadingTopic;
use postcard_rpc::server::Sender;

use crate::{
    actuator::Actuator,
    messages::{Command, Event},
    rpc::{AppServer, AppTx},
    sensor::Sensor,
};

// Core 0.

#[embassy_executor::task]
pub async fn run_sensor(sensor: Sensor, publisher: DynPublisher<'static, Event>) -> ! {
    sensor.run(publisher).await
}

#[embassy_executor::task]
pub async fn run_actuator(actuator: Actuator, subscriber: DynSubscriber<'static, Command>, publisher: DynPublisher<'static, Event>) -> ! {
    actuator.run(subscriber, publisher).await
}

// Core 1.

#[embassy_executor::task]
pub async fn run_usb_device(mut usb_device: UsbDevice<'static, usb::Driver<'static, USB>>) -> ! {
    usb_device.run().await
}

#[embassy_executor::task]
pub async fn run_rpc_server(mut server: AppServer) -> ! {
    loop {
        // Returns when the host goes away.
        let _ = server.run().await;
        debug!("Host disconnected");
    }
}

#[embassy_executor::task]
pub async fn publish_readings(sender: Sender<AppTx>, mut subscriber: DynSubscriber<'static, Event>) -> ! {
    let mut seq = 0u32;
    loop {
        if let Event::Reading(reading) = subscriber.next_message_pure().await {
            // Fails while no host is listening, then the reading is just dropped.
            if sender.publish::<ReadingTopic>(seq.into(), &reading).await.is_err() {
                trace!("Dropped reading");
            }
            seq = seq.wrapping_add(1);
        }
    }
}
