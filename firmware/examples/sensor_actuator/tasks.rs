use defmt::trace;
use embassy_sync::pubsub::{DynPublisher, DynSubscriber};
use picolink_firmware::link::{Server, Tx};
use picolink_protocol::sensor_actuator::ReadingTopic;
use postcard_rpc::server::Sender;

use crate::{
    actuator::Actuator,
    messages::{Command, Event},
    rpc::RpcApp,
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
pub async fn serve(server: Server<RpcApp>) -> ! {
    server.run().await
}

#[embassy_executor::task]
pub async fn publish_readings(sender: Sender<Tx>, mut subscriber: DynSubscriber<'static, Event>) -> ! {
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
