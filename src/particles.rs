//! Particle systems: what one is made of, read from its JSON with anything unknown refused,
//! and a simulation that comes out the same for the same system and time, so a frame can be
//! redrawn or a paused scene resumed without a jump.

mod audio;
mod control;
mod noise;
mod simulation;
mod system;

pub use audio::Hearing;
pub use control::Control;
pub use simulation::{Particle, Simulation};
pub use system::{
    Between, Child, Emitter, MOST, Operator, Renderer, Shape, Sheet, Spawn, Swing, System, Tie,
    Turbulent,
};
