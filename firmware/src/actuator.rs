//! The Pico's onboard LED.

use defmt::debug;
use embassy_rp::gpio::{Level, Output};
use embassy_sync::pubsub::{DynPublisher, DynSubscriber};
use picolink_protocol::ActuatorState;

use crate::messages::{Command, Event};

pub struct Actuator {
    led: Output<'static>,
}

impl Actuator {
    pub fn new(led: Output<'static>) -> Self {
        Self { led }
    }

    pub async fn run(mut self, mut subscriber: DynSubscriber<'static, Command>, publisher: DynPublisher<'static, Event>) -> ! {
        loop {
            match subscriber.next_message_pure().await {
                Command::SetActuator(state) => {
                    debug!("Setting actuator {}", state);
                    self.set(state);
                }
                Command::GetActuator => {}
            }
            publisher.publish_immediate(Event::Actuator(self.state()));
        }
    }

    fn set(&mut self, state: ActuatorState) {
        self.led.set_level(match state {
            ActuatorState::Off => Level::Low,
            ActuatorState::On => Level::High,
        });
    }

    fn state(&self) -> ActuatorState {
        if self.led.is_set_high() {
            ActuatorState::On
        } else {
            ActuatorState::Off
        }
    }
}
