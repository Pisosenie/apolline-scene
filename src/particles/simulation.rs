use std::collections::HashMap;
use std::sync::Arc;

use super::noise;
use super::system::{Between, Operator, Shape, Sheet, Swing, System, Tie, Turbulent};

/// Simulated in steps of a sixtieth of a second, whatever the frame rate.
const RATE: f64 = 60.0;
const STEP: f32 = 1.0 / RATE as f32;
/// Particle steps a system may take warming up.
const WARMUP_WORK: u64 = 20_000_000;
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
    /// What its oscillations and turbulence draw their own numbers from.
    seed: u64,
    /// Which of the system's particles it is, counting from the start.
    serial: u64,
    /// The parent's particle it was let out from, for a system that follows them.
    leader: Option<u64>,
}

impl Particle {
    /// Where it's heading and how fast, in the system's units a second.
    pub fn velocity(&self) -> [f32; 3] {
        self.velocity
    }

    /// Seconds since it was let out.
    pub fn age(&self) -> f32 {
        self.age
    }

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
    /// Steps taken since it started, warming up included, which turbulence moves with.
    taken: u64,
    /// Where the pointer is in the system's own units, for control points locked to it.
    pointer: Option<[f32; 2]>,
    /// What scripts scale the system's instance overrides by as it runs.
    tuning: Tuning,
    /// The most particles there's room for, whatever its count is tuned to.
    most: usize,
    /// What share of its rate each emitter lets out at how loud the music is now.
    heard: Vec<f32>,
    /// What share of its speed a turbulent start gives at how loud the music is now.
    stirred: f32,
    /// Particles let out so far, which numbers the next.
    serial: u64,
    /// The parent system run alongside, for one its particles let out, and when they do.
    leader: Option<(Box<Simulation>, Tie)>,
    /// Where each of the parent's particles was a step ago and what its emitters owe, by its
    /// serial.
    following: HashMap<u64, Follow>,
}

struct Follow {
    at: [f32; 3],
    owed: Vec<f32>,
}

/// How much more of each instance override than the system started with.
#[derive(Clone, Copy, Debug)]
struct Tuning {
    count: f32,
    rate: f32,
    life: f32,
    size: f32,
    alpha: f32,
    speed: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            count: 1.0,
            rate: 1.0,
            life: 1.0,
            size: 1.0,
            alpha: 1.0,
            speed: 1.0,
        }
    }
}

impl Simulation {
    /// Starts the system as the scene finds it, already `warmup` seconds in.
    pub fn new(system: Arc<System>, seed: u64) -> Self {
        let mut simulation = Simulation::cold(system, seed);
        simulation.restart();
        simulation
    }

    /// The system before it starts: `restart` starts it.
    fn cold(system: Arc<System>, seed: u64) -> Self {
        let silence = [[0.0; 64]; 2];
        let heard = system
            .emitters
            .iter()
            .map(|emitter| {
                emitter
                    .hearing
                    .map_or(1.0, |hearing| hearing.share(&silence))
            })
            .collect();
        let stirred = system
            .spawn
            .turbulent
            .and_then(|turbulent| turbulent.hearing)
            .map_or(1.0, |hearing| hearing.share(&silence));
        Simulation {
            heard,
            stirred,
            owed: vec![0.0; system.emitters.len()],
            system,
            seed,
            random: 0,
            particles: Vec::new(),
            elapsed: 0.0,
            steps: 0,
            taken: 0,
            pointer: None,
            tuning: Tuning::default(),
            most: usize::MAX,
            serial: 0,
            leader: None,
            following: HashMap::new(),
        }
    }

    /// The same system as it would run beside another, not yet warmed up: following it warms
    /// it up.
    pub fn twin(&self) -> Simulation {
        let mut twin = Simulation::cold(Arc::clone(&self.system), self.seed);
        twin.most = self.most;
        twin.leader = (self.leader.as_ref()).map(|(leader, tie)| (Box::new(leader.twin()), *tie));
        twin
    }

    /// Lets its particles out from each of `leader`'s instead of its own origin, running it
    /// alongside; started again from there.
    pub fn follow(&mut self, leader: Simulation, tie: Tie) {
        self.leader = Some((Box::new(leader), tie));
        self.restart();
    }

    /// Back to how it stood when the scene started.
    pub fn restart(&mut self) {
        self.reset();
        for _ in 0..self.warmup_steps() {
            self.step();
        }
    }

    fn warmup_steps(&self) -> u64 {
        // A follower warms up as long as the system it follows does, which it moves with.
        let warmup = (f64::from(self.warmup()) * RATE).round() as u64;
        // Warming up happens before the first frame, in one go: a system that could hold many
        // particles warms up for less time rather than holding the host up.
        warmup.min(WARMUP_WORK / self.work().max(1) as u64)
    }

    /// Back to the start, before any warming up.
    fn reset(&mut self) {
        self.random = self.seed ^ 0x9E37_79B9_7F4A_7C15;
        self.particles.clear();
        self.owed.iter_mut().for_each(|owed| *owed = 0.0);
        self.elapsed = 0.0;
        self.steps = 0;
        self.taken = 0;
        self.serial = 0;
        self.following.clear();
        if let Some((leader, _)) = &mut self.leader {
            leader.reset();
            return;
        }
        for at in 0..self.system.emitters.len() {
            let free = self.room().saturating_sub(self.particles.len());
            for _ in 0..self.system.emitters[at].burst.min(free) {
                self.emit(at, [0.0; 3], None);
            }
        }
    }

    fn warmup(&self) -> f32 {
        self.leader
            .as_ref()
            .map_or(self.system.warmup, |(leader, _)| leader.warmup())
    }

    /// Lets a particle out of emitter `at`, `from` away from where it would be, if there's
    /// room.
    fn emit(&mut self, at: usize, from: [f32; 3], leader: Option<u64>) {
        if self.particles.len() >= self.room() {
            return;
        }
        let mut particle = self.spawn(at);
        particle.position = [0, 1, 2].map(|axis| particle.position[axis] + from[axis]);
        particle.leader = leader;
        self.particles.push(particle);
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

    /// Whether a control point an operator uses follows the pointer.
    pub fn points(&self) -> bool {
        self.system.operators.iter().any(|operator| match operator {
            Operator::Attract { control, .. } => self.system.controls[*control].pointer,
            _ => false,
        }) || self
            .leader
            .as_ref()
            .is_some_and(|(leader, _)| leader.points())
    }

    /// Whether an emitter lets particles out as loud as the music is.
    pub fn listens(&self) -> bool {
        self.system
            .emitters
            .iter()
            .any(|emitter| emitter.hearing.is_some())
            || self
                .system
                .spawn
                .turbulent
                .is_some_and(|turbulent| turbulent.hearing.is_some())
            || self
                .leader
                .as_ref()
                .is_some_and(|(leader, _)| leader.listens())
    }

    /// The music is this loud now, left and right, in 64 bins from low to high.
    pub fn hear(&mut self, spectrum: &[[f32; 64]; 2]) {
        for (heard, emitter) in self.heard.iter_mut().zip(&self.system.emitters) {
            if let Some(hearing) = emitter.hearing {
                *heard = hearing.share(spectrum);
            }
        }
        if let Some(hearing) = self
            .system
            .spawn
            .turbulent
            .and_then(|turbulent| turbulent.hearing)
        {
            self.stirred = hearing.share(spectrum);
        }
        if let Some((leader, _)) = &mut self.leader {
            leader.hear(spectrum);
        }
    }

    /// Room for `most` particles, which a tuned count stays within.
    pub fn allow(&mut self, most: usize) {
        self.most = most;
    }

    /// Scales instance override `key` by `factor` from what the system's file says, unless
    /// the system leaves that override out.
    pub fn tune(&mut self, key: &str, factor: f32) {
        if !self.system.overridable(key) {
            return;
        }
        let factor = if factor.is_finite() {
            factor.clamp(0.0, 1000.0)
        } else {
            1.0
        };
        let tuning = &mut self.tuning;
        match key {
            "count" => tuning.count = factor,
            "rate" => tuning.rate = factor,
            "lifetime" => tuning.life = factor,
            "size" => tuning.size = factor,
            "alpha" => tuning.alpha = factor,
            "speed" => tuning.speed = factor,
            _ => {}
        }
    }

    /// How many particles may be alive now.
    /// The most particles one step can move: its own and every system it follows.
    fn work(&self) -> usize {
        self.room() + self.leader.as_ref().map_or(0, |(leader, _)| leader.work())
    }

    fn room(&self) -> usize {
        ((self.system.max as f32 * self.tuning.count) as usize).min(self.most)
    }

    /// The pointer moved to `at` in the system's own units.
    pub fn point(&mut self, at: [f32; 2]) {
        self.pointer = Some(at);
        if let Some((leader, _)) = &mut self.leader {
            leader.point(at);
        }
    }

    /// How its particles are drawn, each once by each.
    pub fn renderers(&self) -> &[crate::particles::Renderer] {
        &self.system.renderers
    }

    /// Whether its sprite sheet's frames show one at a time, not blended into the next.
    pub fn crisp(&self) -> bool {
        self.system.crisp
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
        if self.leader.is_some() {
            self.follow_leader();
        } else {
            for at in 0..system.emitters.len() {
                let mut owed = self.owed[at];
                self.emit_owed(at, &mut owed, [0.0; 3], None);
                self.owed[at] = owed;
            }
        }
        let controls = system
            .controls
            .map(|control| match (control.pointer, self.pointer) {
                (true, Some([x, y])) => [
                    x + control.offset[0],
                    y + control.offset[1],
                    control.offset[2],
                ],
                _ => control.offset,
            });
        let world = World {
            seed: self.seed,
            now: self.taken as f64 / RATE,
            controls,
        };
        for particle in &mut self.particles {
            live(particle, &system.operators, world);
        }
        self.taken += 1;
    }

    /// Lets out what emitter `at` owes this step, `from` away; whatever it owes past the room
    /// there is goes unspent, however fast it asks.
    fn emit_owed(&mut self, at: usize, owed: &mut f32, from: [f32; 3], leader: Option<u64>) {
        let emitter = &self.system.emitters[at];
        *owed += emitter.rate * self.tuning.rate * self.heard[at] * STEP;
        let due = owed.floor();
        *owed -= due;
        let free = self.room().saturating_sub(self.particles.len());
        for _ in 0..(due as usize).min(free) {
            self.emit(at, from, leader);
        }
    }

    /// Steps the system it follows, then lets particles out from each of its: a burst from
    /// each one born, and its rate from each while it lives. Unless they stay where they were
    /// let out, they move with the particle they came from and go when it does.
    fn follow_leader(&mut self) {
        let Some((leader, tie)) = self.leader.as_mut() else {
            return;
        };
        let tie = *tie;
        leader.step();
        let leaders: Vec<(u64, [f32; 3])> = leader
            .particles
            .iter()
            .map(|particle| (particle.serial, particle.position))
            .collect();
        let mut following = std::mem::take(&mut self.following);
        let emitters = self.system.emitters.len();
        // Never more turns than there is room: a full system lets nothing more out.
        let burst = |simulation: &mut Simulation, at: [f32; 3], serial: Option<u64>| {
            for emitter in 0..emitters {
                let free = simulation.room().saturating_sub(simulation.particles.len());
                if free == 0 {
                    return;
                }
                for _ in 0..simulation.system.emitters[emitter].burst.min(free) {
                    simulation.emit(emitter, at, serial);
                }
            }
        };
        if tie != Tie::Follow {
            // A burst where each was born, or where each that's gone was last; what comes out
            // goes its own way.
            let mut next = HashMap::with_capacity(leaders.len());
            for (serial, at) in leaders {
                if following.remove(&serial).is_none() && tie == Tie::Spawn {
                    burst(self, at, None);
                }
                next.insert(
                    serial,
                    Follow {
                        at,
                        owed: Vec::new(),
                    },
                );
            }
            if tie == Tie::Death {
                let mut gone: Vec<(u64, [f32; 3])> = following
                    .iter()
                    .map(|(serial, follow)| (*serial, follow.at))
                    .collect();
                gone.sort_by_key(|(serial, _)| *serial);
                for (_, at) in gone {
                    burst(self, at, None);
                }
            }
            self.following = next;
            return;
        }
        let moved: HashMap<u64, [f32; 3]> = leaders
            .iter()
            .filter_map(|(serial, at)| {
                let before = following.get(serial)?.at;
                Some((*serial, [0, 1, 2].map(|axis| at[axis] - before[axis])))
            })
            .collect();
        if !self.system.world {
            self.particles.retain_mut(|particle| {
                let Some(serial) = particle.leader else {
                    return true;
                };
                let Some(by) = moved.get(&serial) else {
                    return false;
                };
                for (position, by) in particle.position.iter_mut().zip(by) {
                    *position += by;
                }
                true
            });
        }
        let mut next = HashMap::with_capacity(leaders.len());
        for (serial, at) in leaders {
            let mut owed = match following.remove(&serial) {
                Some(follow) => follow.owed,
                None => {
                    burst(self, at, Some(serial));
                    vec![0.0; emitters]
                }
            };
            for (emitter, owed) in owed.iter_mut().enumerate() {
                self.emit_owed(emitter, owed, at, Some(serial));
            }
            next.insert(serial, Follow { at, owed });
        }
        self.following = next;
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
        let tuning = self.tuning;
        let size = self.between(spawn.size) * tuning.size;
        let alpha = self.between(spawn.alpha) * tuning.alpha;
        let color = self.along(spawn.color);
        let rotation = self.within(spawn.rotation);
        let born = Look { size, alpha, color };
        let mut velocity = self.within(spawn.velocity);
        if let Some(turbulent) = spawn.turbulent {
            let push = self.turbulent(turbulent);
            velocity = [0, 1, 2].map(|axis| velocity[axis] + push[axis]);
        }
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
        let velocity = velocity.map(|velocity| velocity * tuning.speed);
        Particle {
            position: [0, 1, 2].map(|axis| emitter.origin[axis] + offset[axis]),
            size,
            rotation: rotation[2],
            color,
            alpha,
            velocity,
            spin: self.within(spawn.spin),
            age: 0.0,
            serial: {
                self.serial += 1;
                self.serial
            },
            leader: None,
            life: (self.between(spawn.life) * tuning.life).max(0.001),
            born,
            // Drawn only for systems that use it, so the rest keep their random sequence.
            pick: if system.sheet == Sheet::Random {
                self.unit()
            } else {
                0.0
            },
            seed: if system.operators.iter().any(|operator| {
                matches!(
                    operator,
                    Operator::Oscillate { .. } | Operator::Turbulence { .. }
                )
            }) {
                self.random = self.random.wrapping_add(0x9E37_79B9_7F4A_7C15);
                self.random
            } else {
                0
            },
        }
    }

    /// A turbulent start: `forward` turned in the scene's plane by its offset and by noise
    /// over when the particle is born.
    fn turbulent(&mut self, turbulent: Turbulent) -> [f32; 3] {
        let phase = self.uniform(turbulent.phase[0], turbulent.phase[1]);
        let speed = self.uniform(turbulent.speed[0], turbulent.speed[1]) * self.stirred;
        let at = self.taken as f64 / RATE * f64::from(turbulent.timescale) + f64::from(phase);
        let swing = noise::noise(self.seed, [at, 0.5, 0.5]);
        let turn = turbulent.offset + turbulent.scale * std::f32::consts::PI * swing;
        let [x, y, z] = turbulent.forward;
        let length = (x * x + y * y + z * z).sqrt();
        if length < 1e-6 {
            return [0.0; 3];
        }
        let (sin, cos) = turn.sin_cos();
        [
            (x * cos - y * sin) / length * speed,
            (x * sin + y * cos) / length * speed,
            z / length * speed,
        ]
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

    /// One point on the line between a range's ends, as a colour is picked: a stock snow that
    /// runs from white to grey falls in greys, not in every colour between.
    fn along(&mut self, range: Between<[f32; 3]>) -> [f32; 3] {
        let along = self.unit().powf(range.exponent);
        [0, 1, 2].map(|axis| range.low[axis] + (range.high[axis] - range.low[axis]) * along)
    }
}

/// What every particle's step shares: the system's seed and how long it has run.
#[derive(Clone, Copy)]
struct World {
    seed: u64,
    now: f64,
    /// Where each control point is in the system's units.
    controls: [[f32; 3]; super::control::CONTROLS],
}

/// One step of a particle's life under the system's operators.
fn live(particle: &mut Particle, operators: &[Operator], world: World) {
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
            Operator::Turbulence {
                scale,
                timescale,
                speed,
                phase,
                mask,
                blend,
            } => {
                let pick = |which: u64, range: [f32; 2]| {
                    range[0] + (range[1] - range[0]) * hashed(particle.seed, at as u64 * 8 + which)
                };
                let scale = f64::from(scale);
                let along = world.now * f64::from(timescale) + f64::from(pick(0, phase));
                let [x, y, z] = particle.position.map(|value| f64::from(value) * scale);
                let push = noise::field(world.seed, [x, y, z + along]);
                let strength = pick(1, speed) * fade(t, blend) * STEP;
                for ((velocity, push), mask) in particle.velocity.iter_mut().zip(push).zip(mask) {
                    *velocity += push * mask * strength;
                }
            }
            Operator::Attract {
                control,
                origin,
                scale,
                threshold,
                blend,
            } => {
                let toward: [f32; 3] = [0, 1, 2].map(|axis| {
                    world.controls[control][axis] + origin[axis] - particle.position[axis]
                });
                let distance = toward.iter().map(|v| v * v).sum::<f32>().sqrt();
                if distance > 1e-3 && distance < threshold {
                    let pull = scale * fade(t, blend) * STEP / distance;
                    for (velocity, toward) in particle.velocity.iter_mut().zip(toward) {
                        *velocity += toward * pull;
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

    fn turbulent(initializer: &str, operator: &str) -> Arc<System> {
        system(&format!(
            r#"{{"material": "m.json", "maxcount": 200,
                "emitter": [{{"name": "sphererandom", "rate": 60}}],
                "initializer": [{{"name": "lifetimerandom", "min": 2, "max": 2}}, {initializer}],
                "operator": [{{"name": "movement"}}{operator}],
                "renderer": [{{"name": "sprite"}}]}}"#
        ))
    }

    #[test]
    fn turbulent_starts_turn_forward_by_their_offset_and_noise() {
        let heading = |initializer: &str| {
            let mut simulation = Simulation::new(turbulent(initializer, ""), 3);
            simulation.advance(1.0);
            simulation
                .particles()
                .iter()
                .map(|particle| {
                    let [x, y, _] = particle.velocity();
                    (y.atan2(x), x.hypot(y))
                })
                .collect::<Vec<_>>()
        };
        // Straight up at their own speed when nothing turns them; a half turn sends them down.
        let up = heading(
            r#"{"name": "turbulentvelocityrandom", "scale": 0, "speedmin": 50, "speedmax": 60}"#,
        );
        assert!(up.iter().all(|&(angle, speed)| {
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-4 && (50.0..=60.0).contains(&speed)
        }));
        let down =
            heading(r#"{"name": "turbulentvelocityrandom", "scale": 0, "offset": 3.14159265}"#);
        assert!(
            down.iter()
                .all(|&(angle, _)| (angle + std::f32::consts::FRAC_PI_2).abs() < 1e-3)
        );
        // Noise sways them within a tenth of a half turn, not all the same way.
        let swaying =
            heading(r#"{"name": "turbulentvelocityrandom", "scale": 0.1, "timescale": 2}"#);
        let angles: Vec<f32> = swaying.iter().map(|&(angle, _)| angle).collect();
        assert!(
            angles
                .iter()
                .all(|angle| (angle - std::f32::consts::FRAC_PI_2).abs()
                    <= 0.1 * std::f32::consts::PI + 1e-4)
        );
        assert!(angles.iter().any(|angle| (angle - angles[0]).abs() > 1e-3));
    }

    #[test]
    fn a_control_point_on_the_pointer_pushes_particles_away_near_it() {
        let system = system(
            r#"{"material": "m.json", "maxcount": 400,
                "controlpoint": [{"id": 1, "flags": 1, "offset": "0 0 0"}],
                "emitter": [{"name": "sphererandom", "rate": 400, "distancemax": 100}],
                "initializer": [{"name": "lifetimerandom", "min": 5, "max": 5}],
                "operator": [{"name": "movement"},
                             {"name": "controlpointattract", "controlpoint": 1, "scale": -5000, "threshold": 60}],
                "renderer": [{"name": "sprite"}]}"#,
        );
        let mut pushed = Simulation::new(Arc::clone(&system), 9);
        assert!(pushed.points());
        pushed.point([0.0, 0.0]);
        pushed.advance(1.0);
        let near = |simulation: &Simulation| {
            let close = simulation
                .particles()
                .iter()
                .filter(|particle| particle.position[0].hypot(particle.position[1]) < 30.0);
            close.count()
        };
        let mut away = Simulation::new(system, 9);
        away.point([5000.0, 0.0]);
        away.advance(1.0);
        assert!(near(&away) > 20, "{}", near(&away));
        assert!(
            near(&pushed) < near(&away) / 4,
            "{} {}",
            near(&pushed),
            near(&away)
        );
    }

    #[test]
    fn scripts_scale_a_running_system_within_its_room() {
        let mut quiet = Simulation::new(system(FOUNTAIN), 7);
        quiet.tune("rate", 0.0);
        quiet.advance(0.5);
        assert!(quiet.particles().is_empty());
        let mut crowded = Simulation::new(system(FOUNTAIN), 7);
        crowded.allow(80);
        crowded.tune("rate", 20.0);
        crowded.tune("count", 10.0);
        crowded.advance(0.9);
        assert_eq!(crowded.particles().len(), 80);
        // However much it's asked for, a step lets out no more than there's room for.
        let flood = FOUNTAIN.replace(r#""rate": 30"#, r#""rate": 100000"#);
        let mut flooded = Simulation::new(system(&flood), 7);
        flooded.allow(80);
        flooded.tune("rate", 1000.0);
        flooded.tune("count", 10.0);
        flooded.advance(1.0);
        assert_eq!(flooded.particles().len(), 80);
        let mut plain = Simulation::new(system(FOUNTAIN), 7);
        plain.advance(0.9);
        assert!(plain.particles().len() < 50);
        // They start twice the size, before the operators shrink them.
        let largest = |simulation: &Simulation| {
            let sizes = simulation.particles().iter().map(|particle| particle.size);
            sizes.fold(0.0f32, f32::max)
        };
        let mut sized = Simulation::new(system(FOUNTAIN), 7);
        sized.tune("size", 2.0);
        sized.advance(0.9);
        assert!((largest(&sized) / largest(&plain) - 2.0).abs() < 1e-3);
    }

    #[test]
    fn an_emitter_that_hears_lets_particles_out_only_to_music() {
        let listening = FOUNTAIN.replace(
            r#""rate": 30,"#,
            r#""rate": 30, "audioprocessingmode": 3, "audioprocessingbounds": "0.2 0.6","#,
        );
        let mut quiet = Simulation::new(system(&listening), 3);
        assert!(quiet.listens());
        quiet.advance(0.5);
        assert!(quiet.particles().is_empty());
        let mut loud = Simulation::new(system(&listening), 3);
        loud.hear(&[[0.8; 64]; 2]);
        loud.advance(0.5);
        let mut plain = Simulation::new(system(FOUNTAIN), 3);
        plain.advance(0.5);
        assert_eq!(loud.particles().len(), plain.particles().len());
    }

    #[test]
    fn a_turbulent_start_that_hears_sets_off_only_to_music() {
        let listening = turbulent(
            r#"{"name": "turbulentvelocityrandom", "audioprocessingmode": 3, "audioprocessingbounds": "0.5 1"}"#,
            "",
        );
        let speeds = |simulation: &Simulation| -> f32 {
            simulation
                .particles()
                .iter()
                .map(|particle| particle.velocity().iter().map(|v| v * v).sum::<f32>())
                .fold(0.0, f32::max)
        };
        let mut quiet = Simulation::new(Arc::clone(&listening), 4);
        assert!(quiet.listens());
        quiet.advance(0.5);
        assert!(!quiet.particles().is_empty() && speeds(&quiet) == 0.0);
        let mut loud = Simulation::new(listening, 4);
        loud.hear(&[[1.0; 64]; 2]);
        loud.advance(0.5);
        assert!(speeds(&loud) > 100.0);
    }

    #[test]
    fn turbulence_pushes_particles_only_along_its_mask() {
        let still = r#"{"name": "velocityrandom", "min": "0 0 0", "max": "0 0 0"}"#;
        let mut simulation = Simulation::new(
            turbulent(
                still,
                r#", {"name": "turbulence", "mask": "1 0 0", "scale": 0.01, "speedmin": 400, "speedmax": 400}"#,
            ),
            5,
        );
        simulation.advance(1.0);
        let particles = simulation.particles();
        assert!(
            particles
                .iter()
                .all(|particle| particle.velocity()[1] == 0.0)
        );
        assert!(
            particles
                .iter()
                .any(|particle| particle.velocity()[0].abs() > 1.0)
        );
        assert!(
            particles
                .iter()
                .all(|particle| particle.velocity()[0].abs() <= 400.0)
        );
        let mut again = Simulation::new(Arc::clone(&simulation.system), 5);
        again.advance(1.0);
        assert!(again.particles() == particles);
    }

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
    fn a_follower_warms_up_within_the_work_its_leader_takes_too() {
        let crowd = system(
            r#"{"material": "m.json", "maxcount": 20000, "starttime": 30,
                "emitter": [{"name": "boxrandom", "rate": 1000, "distancemax": "0 0 0"}],
                "initializer": [{"name": "lifetimerandom", "min": 60, "max": 60}],
                "renderer": [{"name": "sprite"}]}"#,
        );
        let few = system(
            r#"{"material": "m.json", "maxcount": 10,
                "emitter": [{"name": "boxrandom", "rate": 1, "distancemax": "0 0 0"}],
                "renderer": [{"name": "sprite"}]}"#,
        );
        let mut follower = Simulation::cold(few, 4);
        follower.leader = Some((Box::new(Simulation::cold(crowd, 3)), Tie::Follow));
        // Thirty seconds are 1800 steps; with its leader's 20,000 particles it takes fewer.
        assert_eq!(follower.warmup_steps(), WARMUP_WORK / 20_010);
    }

    #[test]
    fn followers_come_out_of_each_particle_and_glow_with_it_or_trail_behind() {
        // One particle a second, flying right at 100 a second for two seconds.
        let star = system(
            r#"{"material": "m.json", "maxcount": 4,
                "emitter": [{"name": "boxrandom", "rate": 1, "distancemax": "0 0 0"}],
                "initializer": [{"name": "lifetimerandom", "min": 2, "max": 2},
                                {"name": "velocityrandom", "min": "100 0 0", "max": "100 0 0"}],
                "operator": [{"name": "movement", "gravity": "0 0 0"}],
                "renderer": [{"name": "sprite"}]}"#,
        );
        let follower = |flags: u32, emitter: &str| {
            system(&format!(
                r#"{{"material": "m.json", "maxcount": 64, "flags": {flags},
                    "emitter": [{{"name": "boxrandom", "origin": "0 10 0", "distancemax": "0 0 0",
                                  {emitter}}}],
                    "initializer": [{{"name": "lifetimerandom", "min": 5, "max": 5}}],
                    "operator": [{{"name": "movement", "gravity": "0 0 0"}}],
                    "renderer": [{{"name": "sprite"}}]}}"#
            ))
        };
        let leader = Simulation::new(Arc::clone(&star), 3);
        let mut glow = Simulation::new(follower(0, r#""rate": 0, "instantaneous": 1"#), 4);
        glow.follow(leader.twin(), Tie::Follow);
        let run = |simulation: &mut Simulation, seconds: f32| {
            for _ in 0..(seconds * 2.0) as usize {
                simulation.advance(0.5);
            }
        };
        run(&mut glow, 1.5);
        // The one star out so far carries its glow, 10 above it.
        assert_eq!(glow.particles().len(), 1);
        let star_at = glow.leader.as_ref().unwrap().0.particles()[0].position;
        let glow_at = glow.particles()[0].position;
        assert!((glow_at[0] - star_at[0]).abs() < 1e-3 && star_at[0] > 40.0);
        assert!((glow_at[1] - star_at[1] - 10.0).abs() < 1e-3);
        // Its glow goes with it when it dies; a later star brings its own.
        run(&mut glow, 1.0);
        let stars = glow.leader.as_ref().unwrap().0.particles().len();
        assert_eq!(glow.particles().len(), stars);

        let mut trail = Simulation::new(follower(1, r#""rate": 30"#), 4);
        trail.follow(leader.twin(), Tie::Follow);
        run(&mut trail, 2.0);
        // A trail stays where it was let out, spread along the way the star went.
        let xs: Vec<f32> = trail.particles().iter().map(|p| p.position[0]).collect();
        assert!(xs.len() > 20);
        assert!(xs.iter().cloned().fold(f32::MAX, f32::min) < 10.0);
        assert!(xs.iter().cloned().fold(f32::MIN, f32::max) > 80.0);
        // The same moment twice is the same particles.
        let before = trail.particles().to_vec();
        trail.seek(2.0);
        assert_eq!(trail.particles(), &before[..]);
    }

    #[test]
    fn colours_are_picked_between_their_ends_not_a_channel_at_a_time() {
        let snow = system(&FOUNTAIN.replace(
            r#"{"name": "lifetimerandom", "min": 1, "max": 1},"#,
            r#"{"name": "lifetimerandom", "min": 1, "max": 1},
               {"name": "colorrandom", "min": "255 255 255", "max": "0 0 0"},"#,
        ));
        let mut simulation = Simulation::new(snow, 2);
        simulation.advance(0.5);
        assert!(simulation.particles().len() > 5);
        for particle in simulation.particles() {
            let [r, g, b] = particle.color;
            assert!(
                (r - g).abs() < 1e-6 && (g - b).abs() < 1e-6,
                "{:?}",
                particle.color
            );
        }
    }

    #[test]
    fn children_burst_where_particles_are_born_and_where_they_die() {
        // A star a second from the start, each living half a second, flying right.
        let star = system(
            r#"{"material": "m.json", "maxcount": 4,
                "emitter": [{"name": "boxrandom", "rate": 1, "distancemax": "0 0 0"}],
                "initializer": [{"name": "lifetimerandom", "min": 0.5, "max": 0.5},
                                {"name": "velocityrandom", "min": "100 0 0", "max": "100 0 0"}],
                "operator": [{"name": "movement", "gravity": "0 0 0"}],
                "renderer": [{"name": "sprite"}]}"#,
        );
        let sparks = system(
            r#"{"material": "m.json", "maxcount": 64,
                "emitter": [{"name": "boxrandom", "rate": 0, "instantaneous": 3, "distancemax": "0 0 0"}],
                "initializer": [{"name": "lifetimerandom", "min": 5, "max": 5}],
                "operator": [{"name": "movement", "gravity": "0 0 0"}],
                "renderer": [{"name": "sprite"}]}"#,
        );
        let leader = Simulation::new(star, 3);
        let run = |tie: Tie, seconds: f32| {
            let mut sparks = Simulation::new(Arc::clone(&sparks), 4);
            sparks.follow(leader.twin(), tie);
            for _ in 0..(seconds * 4.0) as usize {
                sparks.advance(0.25);
            }
            sparks
        };
        // Born at a second and gone at a second and a half, fifty to the right.
        assert_eq!(run(Tie::Spawn, 1.25).particles().len(), 3);
        assert!(run(Tie::Death, 1.25).particles().is_empty());
        let dead = run(Tie::Death, 1.75);
        assert_eq!(dead.particles().len(), 3);
        assert!(
            dead.particles()
                .iter()
                .all(|spark| (spark.position[0] - 50.0).abs() < 2.0)
        );
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
