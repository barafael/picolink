//! Publishes the chip temperature at a fixed interval.

use defmt::{trace, warn};
use embassy_sync::pubsub::DynPublisher;
use embassy_time::{Duration, Instant, Ticker};
use picolink_firmware::temperature::TemperatureSensor;
use picolink_protocol::sensor_actuator::SensorReading;

use crate::messages::Event;

const SENSOR_INTERVAL: Duration = Duration::from_millis(500);

pub struct Sensor {
    temperature: TemperatureSensor,
}

impl Sensor {
    pub fn new(temperature: TemperatureSensor) -> Self {
        Self { temperature }
    }

    pub async fn run(mut self, publisher: DynPublisher<'static, Event>) -> ! {
        let mut ticker = Ticker::every(SENSOR_INTERVAL);
        loop {
            ticker.next().await;
            match self.temperature.read_celsius().await {
                Ok(celsius) => {
                    let reading = SensorReading {
                        uptime_ms: Instant::now().as_millis(),
                        celsius,
                    };
                    trace!("{}", reading);
                    publisher.publish_immediate(Event::Reading(reading));
                }
                Err(e) => warn!("Failed to read sensor: {}", e),
            }
        }
    }
}
