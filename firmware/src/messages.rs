//! What the cores send each other. Not part of the wire protocol.

use picolink_protocol::{ActuatorState, SensorReading};

/// Core 1 to core 0.
#[derive(Debug, Clone, Copy, defmt::Format)]
pub enum Command {
    SetActuator(ActuatorState),
    GetActuator,
}

/// Core 0 to core 1.
#[derive(Debug, Clone, Copy, defmt::Format)]
pub enum Event {
    /// The actuator's state, in answer to every [`Command`].
    Actuator(ActuatorState),
    Reading(SensorReading),
}
