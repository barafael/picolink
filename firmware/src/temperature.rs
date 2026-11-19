//! The RP2040's internal temperature sensor.

use embassy_rp::adc::{self, Adc};

/// One LSB is about half a degree, so each reading averages this many samples.
const OVERSAMPLING: u32 = 16;

pub struct TemperatureSensor {
    adc: Adc<'static, adc::Async>,
    channel: adc::Channel<'static>,
}

impl TemperatureSensor {
    /// `channel` should be `adc::Channel::new_temp_sensor(p.ADC_TEMP_SENSOR)`.
    pub fn new(adc: Adc<'static, adc::Async>, channel: adc::Channel<'static>) -> Self {
        Self { adc, channel }
    }

    /// The chip temperature. Uncalibrated, so expect it to be off by a few degrees.
    pub async fn read_celsius(&mut self) -> Result<f32, adc::Error> {
        let mut sum = 0;
        for _ in 0..OVERSAMPLING {
            sum += u32::from(self.adc.read(&mut self.channel).await?);
        }
        Ok(to_celsius(sum as f32 / OVERSAMPLING as f32))
    }
}

/// RP2040 datasheet, section 4.9.5 "Temperature Sensor".
fn to_celsius(raw: f32) -> f32 {
    let volts = raw * 3.3 / 4096.0;
    27.0 - (volts - 0.706) / 0.001721
}
