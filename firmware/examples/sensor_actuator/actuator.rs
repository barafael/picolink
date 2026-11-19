//! The Pico's onboard LED.

use defmt::debug;
use embassy_rp::gpio::{Level, Output};
use embassy_sync::pubsub::{DynPublisher, DynSubscriber};
use picolink_protocol::sensor_actuator::ActuatorState;

use crate::messages::{Command, Event};

/// Answers every [`Command`] with the LED's state.
#[embassy_executor::task]
pub async fn run(mut led: Output<'static>, mut subscriber: DynSubscriber<'static, Command>, publisher: DynPublisher<'static, Event>) -> ! {
    loop {
        match subscriber.next_message_pure().await {
            Command::SetActuator(state) => {
                debug!("Setting actuator {}", state);
                led.set_level(match state {
                    ActuatorState::Off => Level::Low,
                    ActuatorState::On => Level::High,
                });
            }
            Command::GetActuator => {}
        }
        let state = if led.is_set_high() { ActuatorState::On } else { ActuatorState::Off };
        publisher.publish_immediate(Event::Actuator(state));
    }
}
