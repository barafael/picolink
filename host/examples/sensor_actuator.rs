//! Talks to the `sensor_actuator` firmware from the command line.
//!
//! ```sh
//! cargo run -p picolink-host --example sensor_actuator -- info
//! cargo run -p picolink-host --example sensor_actuator -- led on        # or off, toggle
//! cargo run -p picolink-host --example sensor_actuator -- watch 10      # readings for 10 s
//! ```
//!
//! `--serial <hex>` picks one device out of several.

use std::{process::ExitCode, time::Duration};

use picolink_host::{DEFAULT_TIMEOUT, DeviceFilter, connect, request, serial_number};
use picolink_protocol::sensor_actuator::{ActuatorState, GetActuatorEndpoint, ReadingTopic, SetActuatorEndpoint, USB_PRODUCT};
use tokio::time::{Instant, timeout_at};

const USAGE: &str = "usage: sensor_actuator [--serial <hex>] (info | led (on|off|toggle) | watch [seconds])";

#[tokio::main]
async fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(mut args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let mut filter = DeviceFilter::new(USB_PRODUCT);
    if let Some(i) = args.iter().position(|arg| arg == "--serial") {
        let serial = args.get(i + 1).ok_or(USAGE)?;
        filter = filter.serial_number(u64::from_str_radix(serial, 16)?);
        args.drain(i..=i + 1);
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let device = connect(&filter).await?;
    let client = &device.client;
    match args.as_slice() {
        ["info"] => {
            println!("Connection:    {}", device.location);
            println!("Serial number: {:016x}", serial_number(client).await?);
            println!(
                "Actuator:      {:?}",
                request::<GetActuatorEndpoint>(client, &(), DEFAULT_TIMEOUT).await?
            );
        }
        ["led", state] => {
            let state = match *state {
                "on" => ActuatorState::On,
                "off" => ActuatorState::Off,
                "toggle" => request::<GetActuatorEndpoint>(client, &(), DEFAULT_TIMEOUT).await?.toggled(),
                _ => return Err(USAGE.into()),
            };
            let state = request::<SetActuatorEndpoint>(client, &state, DEFAULT_TIMEOUT).await?;
            println!("Actuator: {state:?}");
        }
        ["watch", rest @ ..] => {
            let seconds = match rest {
                [] => 10,
                [seconds] => seconds.parse()?,
                _ => return Err(USAGE.into()),
            };
            let mut readings = client.subscribe_multi::<ReadingTopic>(16).await?;
            let end = Instant::now() + Duration::from_secs(seconds);
            while let Ok(reading) = timeout_at(end, readings.recv()).await {
                let reading = reading?;
                println!("{:>10} ms  {:5.1} °C", reading.uptime_ms, reading.celsius);
            }
        }
        _ => return Err(USAGE.into()),
    }
    client.close();
    Ok(())
}
