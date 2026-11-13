//! The RP2040's internal temperature sensor.

use defmt::{trace, warn};
use embassy_rp::adc::{self, Adc};
use embassy_sync::pubsub::DynPublisher;
use embassy_time::{Instant, Ticker};
use picolink_protocol::SensorReading;

use crate::{constants::SENSOR_INTERVAL, messages::Event};

pub struct Sensor {
    adc: Adc<'static, adc::Async>,
    channel: adc::Channel<'static>,
}

impl Sensor {
    pub fn new(adc: Adc<'static, adc::Async>, channel: adc::Channel<'static>) -> Self {
        Self { adc, channel }
    }

    pub async fn run(mut self, publisher: DynPublisher<'static, Event>) -> ! {
        let mut ticker = Ticker::every(SENSOR_INTERVAL);
        loop {
            ticker.next().await;
            match self.read().await {
                Ok(reading) => {
                    trace!("{}", reading);
                    publisher.publish_immediate(Event::Reading(reading));
                }
                Err(e) => warn!("Failed to read sensor: {}", e),
            }
        }
    }

    async fn read(&mut self) -> Result<SensorReading, adc::Error> {
        // One LSB is about half a degree, so average out the noise.
        let mut sum = 0;
        for _ in 0..OVERSAMPLING {
            sum += u32::from(self.adc.read(&mut self.channel).await?);
        }
        Ok(SensorReading {
            uptime_ms: Instant::now().as_millis(),
            celsius: to_celsius(sum as f32 / OVERSAMPLING as f32),
        })
    }
}

const OVERSAMPLING: u32 = 16;

/// RP2040 datasheet, section 4.9.5 "Temperature Sensor".
fn to_celsius(raw: f32) -> f32 {
    let volts = raw * 3.3 / 4096.0;
    27.0 - (volts - 0.706) / 0.001721
}
