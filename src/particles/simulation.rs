use std::sync::Arc;

use super::system::{Between, Operator, Shape, Sheet, Swing, System};

/// Simulated in steps of a sixtieth of a second, whatever the frame rate.
const RATE: f64 = 60.0;
const STEP: f32 = 1.0 / RATE as f32;
/// A host that falls further behind than this many steps catches up only this far.
const CATCH_UP: u64 = 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    pub position: [f32; 3],
    pub size: f32,
    /// Around the view axis, in radians.
    pub rotation: f32,
    pub color: [f32; 3],
    pub alpha: f32,
    velocity: [f32; 3],
    spin: [f32; 3],
    age: f32,
    life: f32,
    born: Look,
    /// The frame of a sprite sheet it shows, 0 to 1, when the system picks one at random.
    pick: f32,
    /// What its oscillations draw their frequency, scale and phase from.
    seed: u64,
}

impl Particle {
    /// How far through a sprite sheet it is, 0 to 1: its frame's place among the frames, and
    /// how far on to the next.
    pub fn sheet(&self, sheet: Sheet) -> f32 {
        match sheet {
            Sheet::Random => self.pick,
            Sheet::Sequence(times) => {
                let through = if self.life > 0.0 {
                    self.age / self.life
                } else {
                    0.0
                };
                (through * times).fract()
            }
        }
    }
}

/// What a particle started with, which the operators scale over its life.
#[derive(Clone, Debug, PartialEq)]
struct Look {
    size: f32,
    alpha: f32,
    color: [f32; 3],
}

pub struct Simulation {
    system: Arc<System>,
    seed: u64,
    random: u64,
    particles: Vec<Particle>,
    /// Fractions of a particle each emitter owes.
    owed: Vec<f32>,
    /// Seconds the host has moved it on, and the steps taken for them.
    elapsed: f64,
    steps: u64,
}

impl Simulation {
    /// Starts the system as the scene finds it, already `warmup` seconds in.
    pub fn new(system: Arc<System>, seed: u64) -> Self {
        let mut simulation = Simulation {
            owed: vec![0.0; system.emitters.len()],
            system,
            seed,
            random: 0,
            particles: Vec::new(),
            elapsed: 0.0,
            steps: 0,
        };
        simulation.restart();
        simulation
    }

    /// Back to how it stood when the scene started.
    pub fn restart(&mut self) {
        self.random = self.seed ^ 0x9E37_79B9_7F4A_7C15;
        self.particles.clear();
        self.owed.iter_mut().for_each(|owed| *owed = 0.0);
        self.elapsed = 0.0;
        self.steps = 0;
        for at in 0..self.system.emitters.len() {
            for _ in 0..self.system.emitters[at].burst {
                if self.particles.len() < self.system.max {
                    let particle = self.spawn(at);
                    self.particles.push(particle);
                }
            }
        }
        let warmup = (f64::from(self.system.warmup) * RATE).round() as u64;
        for _ in 0..warmup {
            self.step();
        }
    }

    /// Moves on by `seconds`, in whole steps, carrying over what's left.
    pub fn advance(&mut self, seconds: f32) {
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        self.elapsed += f64::from(seconds);
        let due = (self.elapsed * RATE + 1e-6).floor() as u64;
        let behind = due.saturating_sub(self.steps);
        for _ in 0..behind.min(CATCH_UP) {
            self.step();
        }
        self.steps = due;
    }

    /// Moves to `seconds` since the scene started, back to the start first if that's earlier
    /// than where it is, so drawing the same moment twice changes nothing.
    pub fn seek(&mut self, seconds: f32) {
        if !seconds.is_finite() {
            return;
        }
        let target = f64::from(seconds.max(0.0));
        if target + 1e-6 < self.elapsed {
            self.restart();
        }
        let ahead = target - self.elapsed;
        if ahead > 0.0 {
            self.advance(ahead as f32);
        }
    }

    /// How its particles show a sprite sheet.
    pub fn sheet(&self) -> Sheet {
        self.system.sheet
    }

    /// Whether its particles are drawn in perspective.
    pub fn perspective(&self) -> bool {
        self.system.perspective
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    fn step(&mut self) {
        let system = Arc::clone(&self.system);
        for particle in &mut self.particles {
            particle.age += STEP;
        }
        self.particles
            .retain(|particle| particle.age < particle.life);
        for (at, emitter) in system.emitters.iter().enumerate() {
            self.owed[at] += emitter.rate * STEP;
            while self.owed[at] >= 1.0 {
                self.owed[at] -= 1.0;
                if self.particles.len() < system.max {
                    let particle = self.spawn(at);
                    self.particles.push(particle);
                }
            }
        }
        for particle in &mut self.particles {
            live(particle, &system.operators);
        }
    }

    fn spawn(&mut self, emitter: usize) -> Particle {
        let system = Arc::clone(&self.system);
        let emitter = &system.emitters[emitter];
        let spawn = &system.spawn;
        let mut offset = [0.0; 3];
        match emitter.shape {
            Shape::Box => {
                for (axis, value) in offset.iter_mut().enumerate() {
                    let distance = self.uniform(emitter.min[axis], emitter.max[axis]);
                    let sign = if self.unit() < 0.5 { -1.0 } else { 1.0 };
                    *value = distance * sign * emitter.directions[axis];
                }
            }
            Shape::Sphere => {
                let mut direction = [0.0; 3];
                for (axis, value) in direction.iter_mut().enumerate() {
                    *value = (self.unit() * 2.0 - 1.0) * emitter.directions[axis];
                }
                let length = direction.iter().map(|v| v * v).sum::<f32>().sqrt();
                let distance = self.uniform(emitter.min[0], emitter.max[0]);
                if length > 1e-6 {
                    offset = direction.map(|v| v / length * distance);
                }
            }
        }
        let size = self.between(spawn.size);
        let alpha = self.between(spawn.alpha);
        let color = self.within(spawn.color);
        let rotation = self.within(spawn.rotation);
        let born = Look { size, alpha, color };
        let mut velocity = self.within(spawn.velocity);
        // A sphere's particles also fly out from its centre.
        if emitter.shape == Shape::Sphere && emitter.speed != [0.0; 2] {
            let length = offset.iter().map(|v| v * v).sum::<f32>().sqrt();
            let speed = self.uniform(emitter.speed[0], emitter.speed[1]);
            if length > 1e-6 {
                for (velocity, offset) in velocity.iter_mut().zip(offset) {
                    *velocity += offset / length * speed;
                }
            }
        }
        Particle {
            position: [0, 1, 2].map(|axis| emitter.origin[axis] + offset[axis]),
            size,
            rotation: rotation[2],
            color,
            alpha,
            velocity,
            spin: self.within(spawn.spin),
            age: 0.0,
            life: self.between(spawn.life),
            born,
            // Drawn only for systems that use it, so the rest keep their random sequence.
            pick: if system.sheet == Sheet::Random {
                self.unit()
            } else {
                0.0
            },
            seed: if system
                .operators
                .iter()
                .any(|operator| matches!(operator, Operator::Oscillate { .. }))
            {
                self.random = self.random.wrapping_add(0x9E37_79B9_7F4A_7C15);
                self.random
            } else {
                0
            },
        }
    }

    /// xorshift64*: small, fast and the same everywhere.
    fn unit(&mut self) -> f32 {
        self.random ^= self.random >> 12;
        self.random ^= self.random << 25;
        self.random ^= self.random >> 27;
        let bits = self.random.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40;
        bits as f32 / (1u64 << 24) as f32
    }

    fn uniform(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    fn between(&mut self, range: Between<f32>) -> f32 {
        let along = self.unit().powf(range.exponent);
        range.low + (range.high - range.low) * along
    }

    fn within(&mut self, range: Between<[f32; 3]>) -> [f32; 3] {
        let mut out = [0.0; 3];
        for (axis, value) in out.iter_mut().enumerate() {
            let along = self.unit().powf(range.exponent);
            *value = range.low[axis] + (range.high[axis] - range.low[axis]) * along;
        }
        out
    }
}

/// One step of a particle's life under the system's operators.
fn live(particle: &mut Particle, operators: &[Operator]) {
    let t = (particle.age / particle.life).clamp(0.0, 1.0);
    let mut size = 1.0;
    let mut alpha = 1.0;
    let mut color = [1.0; 3];
    for (at, operator) in operators.iter().enumerate() {
        match *operator {
            Operator::Oscillate {
                what,
                frequency,
                scale,
                phase,
                mask,
                blend,
            } => {
                let pick = |which: u64, range: [f32; 2]| {
                    range[0] + (range[1] - range[0]) * hashed(particle.seed, at as u64 * 8 + which)
                };
                let swing = |which: u64, t: f32| {
                    let turn = std::f32::consts::TAU * pick(which, frequency) * t;
                    (turn + pick(which + 1, phase)).sin()
                };
                let weight = fade(t, blend);
                match what {
                    Swing::Alpha | Swing::Size => {
                        let value = scale[0] + (scale[1] - scale[0]) * (0.5 + 0.5 * swing(0, t));
                        let value = 1.0 + (value - 1.0) * weight;
                        if what == Swing::Alpha {
                            alpha *= value;
                        } else {
                            size *= value;
                        }
                    }
                    Swing::Place => {
                        // Moved by what the swing moved this step, on top of any other motion.
                        let before = ((particle.age - STEP) / particle.life).clamp(0.0, 1.0);
                        let reach = pick(6, scale);
                        for (axis, (position, mask)) in
                            particle.position.iter_mut().zip(mask).enumerate()
                        {
                            let which = 2 * axis as u64;
                            let offset = |t: f32| mask * reach * swing(which, t) * fade(t, blend);
                            *position += offset(t) - offset(before);
                        }
                    }
                }
            }
            Operator::Movement { gravity, drag } => {
                let keep = (1.0 - drag * STEP).max(0.0);
                let Particle {
                    velocity, position, ..
                } = particle;
                for ((velocity, position), gravity) in
                    velocity.iter_mut().zip(position).zip(gravity)
                {
                    *velocity = (*velocity + gravity * STEP) * keep;
                    *position += *velocity * STEP;
                }
            }
            Operator::AngularMovement { force, drag } => {
                let keep = (1.0 - drag * STEP).max(0.0);
                for (spin, force) in particle.spin.iter_mut().zip(force) {
                    *spin = (*spin + force * STEP) * keep;
                }
                particle.rotation += particle.spin[2] * STEP;
            }
            Operator::AlphaFade { fade_in, fade_out } => {
                if t < fade_in && fade_in > 0.0 {
                    alpha *= t / fade_in;
                } else if t > fade_out && fade_out < 1.0 {
                    alpha *= (1.0 - t) / (1.0 - fade_out);
                }
            }
            Operator::SizeChange { times, values } => size *= change(t, times, values),
            Operator::AlphaChange { times, values } => alpha *= change(t, times, values),
            Operator::ColorChange { times, values } => {
                for (channel, value) in color.iter_mut().enumerate() {
                    *value *= change(t, times, [values[0][channel], values[1][channel]]);
                }
            }
        }
    }
    particle.size = particle.born.size * size;
    particle.alpha = (particle.born.alpha * alpha).clamp(0.0, 1.0);
    particle.color = [0, 1, 2].map(|channel| particle.born.color[channel] * color[channel]);
}

/// How much of an operator is at work a share `t` of the way through a life, fading in over
/// the first two of `blend` and out over the last two.
fn fade(t: f32, blend: [f32; 4]) -> f32 {
    let rise = |from: f32, to: f32| {
        if to - from < 1e-6 {
            if t >= from { 1.0 } else { 0.0 }
        } else {
            ((t - from) / (to - from)).clamp(0.0, 1.0)
        }
    };
    let out = if blend[2] >= 1.0 {
        1.0
    } else {
        1.0 - rise(blend[2], blend[3])
    };
    rise(blend[0], blend[1]) * out
}

/// A number from 0 to 1 that `seed` and `which` always give: splitmix64.
fn hashed(seed: u64, which: u64) -> f32 {
    let mut z = seed ^ which.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

/// From the first value to the second between two shares of a life, held either side.
fn change(t: f32, times: [f32; 2], values: [f32; 2]) -> f32 {
    let span = times[1] - times[0];
    let along = if span.abs() < 1e-6 {
        if t < times[0] { 0.0 } else { 1.0 }
    } else {
        ((t - times[0]) / span).clamp(0.0, 1.0)
    };
    values[0] + (values[1] - values[0]) * along
}

#[cfg(test)]
mod tests {
    use super::*;

    fn system(json: &str) -> Arc<System> {
        Arc::new(System::parse(json).unwrap())
    }

    const FOUNTAIN: &str = r#"{
        "material": "m.json", "maxcount": 50,
        "emitter": [{"name": "sphererandom", "rate": 30, "distancemax": 10, "origin": "100 100 0"}],
        "initializer": [{"name": "lifetimerandom", "min": 1, "max": 1},
                        {"name": "velocityrandom", "min": "0 100 0", "max": "0 100 0"}],
        "operator": [{"name": "movement", "gravity": "0 -100 0"},
                     {"name": "alphafade", "fadeintime": 0.5, "fadeouttime": 0.5},
                     {"name": "sizechange", "startvalue": 1, "endvalue": 0}],
        "renderer": [{"name": "sprite"}]
    }"#;

    #[test]
    fn the_same_system_and_time_give_the_same_particles() {
        let mut one = Simulation::new(system(FOUNTAIN), 7);
        let mut two = Simulation::new(system(FOUNTAIN), 7);
        for _ in 0..40 {
            one.advance(1.0 / 60.0);
        }
        two.advance(0.4);
        two.advance(40.0 / 60.0 - 0.4);
        assert!(
            one.particles() == two.particles(),
            "frame rate changed the result"
        );
        assert!(!one.particles().is_empty());
        let mut other = Simulation::new(system(FOUNTAIN), 8);
        other.advance(40.0 / 60.0);
        assert!(
            one.particles() != other.particles(),
            "the seed changed nothing"
        );
    }

    #[test]
    fn particles_live_their_lifetime_under_the_operators() {
        let mut simulation = Simulation::new(system(FOUNTAIN), 1);
        simulation.advance(0.5);
        let count = simulation.particles().len();
        assert!((14..=16).contains(&count), "{count}");
        // The first born is about half way: risen and slowed, fully faded in, half its size.
        let first = &simulation.particles()[0];
        let (height, alpha, size) = (first.position[1] - 100.0, first.alpha, first.size);
        assert!(height > 25.0 && height < 50.0, "{height}");
        assert!(alpha > 0.9 && size < 12.0 && size > 8.0, "{alpha} {size}");
        for particle in simulation.particles() {
            let distance = (particle.position[0] - 100.0).abs();
            assert!(distance <= 10.0 + 1e-3);
        }
        simulation.advance(0.9);
        simulation.advance(0.9);
        assert!(simulation.particles().len() <= 50);
        assert!(
            simulation
                .particles()
                .iter()
                .all(|particle| particle.age < particle.life)
        );
    }

    #[test]
    fn a_burst_comes_out_at_once_and_spheres_throw_outward() {
        let burst = system(&FOUNTAIN.replace(
            r#""rate": 30, "distancemax": 10"#,
            r#""rate": 0, "instantaneous": 5, "speedmin": 50, "speedmax": 50, "distancemin": 10, "distancemax": 10"#,
        ));
        let simulation = Simulation::new(burst, 1);
        assert_eq!(simulation.particles().len(), 5);
        for particle in simulation.particles() {
            let out = [particle.position[0] - 100.0, particle.position[1] - 100.0];
            let along = out[0] * particle.velocity[0] + out[1] * (particle.velocity[1] - 100.0);
            assert!(along > 0.0, "a particle flew inward");
        }
    }

    #[test]
    fn a_warmed_up_system_starts_where_a_run_would_be() {
        let warm =
            system(&FOUNTAIN.replace(r#""maxcount": 50"#, r#""maxcount": 50, "starttime": 0.5"#));
        let warmed = Simulation::new(warm, 3);
        let mut run = Simulation::new(system(FOUNTAIN), 3);
        run.advance(0.5);
        assert!(
            warmed.particles() == run.particles(),
            "warming up isn't running"
        );
        let mut restarted = Simulation::new(system(FOUNTAIN), 3);
        restarted.advance(0.7);
        restarted.restart();
        restarted.advance(0.5);
        assert!(
            restarted.particles() == run.particles(),
            "restarting kept something"
        );
    }

    #[test]
    fn particles_swing_their_alpha_and_place_each_their_own_way() {
        let swinging = system(
            r#"{
            "material": "m.json", "maxcount": 50,
            "emitter": [{"name": "boxrandom", "rate": 30, "origin": "0 0 0", "distancemin": "0 0 0", "distancemax": "0 0 0"}],
            "initializer": [{"name": "lifetimerandom", "min": 2, "max": 2}],
            "operator": [{"name": "oscillatealpha", "frequencymin": 2, "frequencymax": 4, "scalemin": 0.25, "scalemax": 1},
                         {"name": "oscillateposition", "frequencymin": 1, "frequencymax": 1, "scalemin": 10, "scalemax": 10, "mask": "1 0 0"}],
            "renderer": [{"name": "sprite"}]
        }"#,
        );
        let mut simulation = Simulation::new(swinging, 3);
        simulation.advance(0.1);
        let mut alphas = Vec::new();
        for _ in 0..60 {
            simulation.advance(1.0 / 60.0);
            let first = &simulation.particles()[0];
            alphas.push(first.alpha);
            // Only x swings, from where it was let out, never further than its reach each way.
            assert!(
                first.position[0].abs() <= 20.0 + 1e-3,
                "{:?}",
                first.position
            );
            assert_eq!(first.position[1], 0.0);
        }
        assert!(
            alphas
                .iter()
                .all(|alpha| (0.25 - 1e-4..=1.0 + 1e-4).contains(alpha))
        );
        let (low, high) = alphas.iter().fold((1.0f32, 0.0f32), |(low, high), alpha| {
            (low.min(*alpha), high.max(*alpha))
        });
        assert!(high - low > 0.5, "the alpha barely moved: {low}..{high}");
        let firsts: Vec<f32> = simulation.particles()[..5]
            .iter()
            .map(|particle| particle.alpha)
            .collect();
        assert!(
            firsts.windows(2).any(|pair| pair[0] != pair[1]),
            "every particle swings alike"
        );
    }

    #[test]
    fn an_operator_fades_in_and_out_over_a_life() {
        assert_eq!(fade(0.5, [0.0, 0.0, 1.0, 1.0]), 1.0);
        assert!((fade(0.1, [0.0, 0.2, 1.0, 1.0]) - 0.5).abs() < 1e-5);
        assert!((fade(0.9, [0.0, 0.0, 0.8, 1.0]) - 0.5).abs() < 1e-5);
        assert_eq!(fade(1.0, [0.0, 0.0, 1.0, 1.0]), 1.0);
    }
}
