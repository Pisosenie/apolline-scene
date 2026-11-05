//! Particle systems: what one is made of, read from its JSON with anything unknown refused,
//! and a simulation that comes out the same for the same system and time, so a frame can be
//! redrawn or a paused scene resumed without a jump.

mod simulation;
mod system;

pub use simulation::{Particle, Simulation};
pub use system::{Between, Emitter, Operator, Shape, Spawn, System};
