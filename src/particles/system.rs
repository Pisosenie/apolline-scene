use serde_json::Value;

use super::control::{self, CONTROLS, Control};
use crate::Error;
use crate::value::{number, text, vector};

/// A particle system as its file describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct System {
    /// The material its sprites are drawn with.
    pub material: String,
    /// The most particles alive at once.
    pub max: usize,
    /// Seconds it has already run when the scene starts.
    pub warmup: f32,
    pub emitters: Vec<Emitter>,
    pub spawn: Spawn,
    pub operators: Vec<Operator>,
    /// Its particles stay where they were let out when the system moves.
    pub world: bool,
    /// Its particles are drawn in perspective: nearer ones bigger and further from the
    /// middle of the scene.
    pub perspective: bool,
    /// How its sprite sheet's frames are shown, when its texture is one.
    pub sheet: Sheet,
    /// Its sprite sheet's frames show one at a time, not blended into the next.
    pub crisp: bool,
    /// The flags of the instance overrides it leaves out.
    pub fixed: u32,
    pub renderer: Renderer,
    /// Systems let out once with it, each placed from its origin.
    pub children: Vec<Child>,
    pub controls: [Control; CONTROLS],
}

/// A child system let out once at its parent's origin, moved, scaled and turned from it, or
/// let out with each of its parent's particles and following it.
#[derive(Clone, Debug, PartialEq)]
pub struct Child {
    /// Its own file, `particles/….json`.
    pub file: String,
    pub origin: [f32; 3],
    pub scale: [f32; 3],
    pub angles: [f32; 3],
    /// How many particles it may have, in place of its own file's count.
    pub max: Option<usize>,
    /// It's let out from each of its parent's particles, `origin` away, as they move.
    pub follow: bool,
}

impl System {
    /// Whether an instance override of `key` changes it.
    pub fn overridable(&self, key: &str) -> bool {
        let flag = match key {
            "color" | "colorn" => 8,
            "count" => 16,
            "lifetime" => 32,
            "size" => 64,
            "speed" => 128,
            _ => 0,
        };
        self.fixed & flag == 0
    }
}

const CHILD: &[&str] = &[
    "angles",
    "controlpointstartindex",
    "flags",
    "maxcount",
    "origin",
    "probability",
    "scale",
    "type",
];

/// How each particle is drawn: a sprite facing the screen, one stretched along the way it
/// goes, `length` times its speed long between `min` and `max` times its size, or a rope
/// behind it along the way it came over the last `length` seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Renderer {
    Sprite,
    Trail { length: f32, min: f32, max: f32 },
    Rope { length: f32 },
}

/// How a particle shows a sprite sheet: a frame picked at random when it's let out, or the
/// frames in turn over its life, this many times over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sheet {
    Random,
    Sequence(f32),
}

/// The system flags that are drawn: worldspace, sprite sheets without frame blending,
/// perspective rendering, and instance overrides of colour, count, lifetime, size and speed
/// left out.
const WORLD: u32 = 1;
const CRISP: u32 = 2;
const PERSPECTIVE: u32 = 4;
const FIXED: u32 = 0xF8;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Within a box around the origin, at least `min` and at most `max` out on each axis.
    Box,
    /// On a shell around the origin, between `min` and `max` from it.
    Sphere,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Emitter {
    pub shape: Shape,
    /// Particles a second.
    pub rate: f32,
    pub origin: [f32; 3],
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// How far each axis is used, 0 flattening it.
    pub directions: [f32; 3],
    /// Speed away from the origin a sphere's particles start with.
    pub speed: [f32; 2],
    /// Particles let out at once when it starts.
    pub burst: usize,
    /// How loud the music has to be for it to let particles out.
    pub hearing: Option<super::audio::Hearing>,
}

/// A value drawn at random between two bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Between<T> {
    pub low: T,
    pub high: T,
    /// Shapes the draw: above 1 favours `low`.
    pub exponent: f32,
}

impl<T: Copy> Between<T> {
    pub fn fixed(value: T) -> Self {
        Between {
            low: value,
            high: value,
            exponent: 1.0,
        }
    }
}

/// What a new particle starts with.
#[derive(Clone, Debug, PartialEq)]
pub struct Spawn {
    pub life: Between<f32>,
    pub size: Between<f32>,
    pub alpha: Between<f32>,
    pub color: Between<[f32; 3]>,
    pub velocity: Between<[f32; 3]>,
    pub rotation: Between<[f32; 3]>,
    pub spin: Between<[f32; 3]>,
    /// Speed added along a direction that noise over time turns.
    pub turbulent: Option<Turbulent>,
}

/// A particle's start along `forward`, turned by `offset` radians and by noise over when it's
/// born, up to `scale` half turns either way, at a speed from `speed`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turbulent {
    pub forward: [f32; 3],
    pub offset: f32,
    pub scale: f32,
    pub speed: [f32; 2],
    /// Added to each particle's place in the noise, which runs `timescale` a second.
    pub phase: [f32; 2],
    pub timescale: f32,
}

impl Default for Spawn {
    fn default() -> Self {
        Spawn {
            life: Between::fixed(1.0),
            size: Between::fixed(20.0),
            alpha: Between::fixed(1.0),
            color: Between::fixed([1.0; 3]),
            velocity: Between::fixed([0.0; 3]),
            rotation: Between::fixed([0.0; 3]),
            spin: Between::fixed([0.0; 3]),
            turbulent: None,
        }
    }
}

/// What happens to particles over their lives. Times are shares of a particle's life.
#[derive(Clone, Debug, PartialEq)]
pub enum Operator {
    Movement {
        gravity: [f32; 3],
        drag: f32,
    },
    AngularMovement {
        force: [f32; 3],
        drag: f32,
    },
    AlphaFade {
        fade_in: f32,
        fade_out: f32,
    },
    SizeChange {
        times: [f32; 2],
        values: [f32; 2],
    },
    AlphaChange {
        times: [f32; 2],
        values: [f32; 2],
    },
    ColorChange {
        times: [f32; 2],
        values: [[f32; 3]; 2],
    },
    /// Each particle swinging its alpha, size or place at a frequency, scale and phase of
    /// its own from these ranges, fading in and out over its life as `blend` says.
    Oscillate {
        what: Swing,
        /// Swings a life.
        frequency: [f32; 2],
        scale: [f32; 2],
        phase: [f32; 2],
        /// How far each axis moves, for places.
        mask: [f32; 3],
        /// Shares of a life where it starts and finishes fading in, and out.
        blend: [f32; 4],
    },
    /// Each particle pushed by noise over its place and time, at a speed of its own from
    /// `speed` each second, as far along each axis as `mask` says.
    Turbulence {
        /// How many noise cells a unit of place crosses, and a second.
        scale: f32,
        timescale: f32,
        speed: [f32; 2],
        phase: [f32; 2],
        mask: [f32; 3],
        blend: [f32; 4],
    },
    /// Particles within `threshold` of a control point, moved by `origin`, sped towards it
    /// by `scale` each second, or away when that's below zero.
    Attract {
        control: usize,
        origin: [f32; 3],
        scale: f32,
        threshold: f32,
        blend: [f32; 4],
    },
}

/// What an oscillation swings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swing {
    Alpha,
    Size,
    Place,
}

const ROOT: &[&str] = &[
    "animationmode",
    "children",
    "controlpoint",
    "emitter",
    "flags",
    "id",
    "initializer",
    "material",
    "maxcount",
    "name",
    "operator",
    "renderer",
    "sequencemultiplier",
    "starttime",
];
const EMITTER: &[&str] = &[
    "directions",
    "distancemax",
    "distancemin",
    "duration",
    "flags",
    "instantaneous",
    "origin",
    "rate",
    "sign",
    "speedmax",
    "speedmin",
];
const RANGE: &[&str] = &["exponent", "max", "min"];
/// The most particles a system has at once.
pub const MOST: usize = 50_000;
const CHANGE: &[&str] = &["endtime", "endvalue", "starttime", "startvalue"];
const TURBULENT: &[&str] = &[
    "forward",
    "offset",
    "phasemax",
    "phasemin",
    "scale",
    "speedmax",
    "speedmin",
    "timescale",
];
const ATTRACT: &[&str] = &[
    "blendinend",
    "blendinstart",
    "blendoutend",
    "blendoutstart",
    "controlpoint",
    "flags",
    "origin",
    "scale",
    "threshold",
];
const TURBULENCE: &[&str] = &[
    "blendinend",
    "blendinstart",
    "blendoutend",
    "blendoutstart",
    "mask",
    "phasemax",
    "phasemin",
    "scale",
    "speedmax",
    "speedmin",
    "timescale",
];

impl System {
    pub fn parse(json: &str) -> Result<Self, Error> {
        let root: Value = serde_json::from_str(json)
            .map_err(|error| Error::Format(format!("particle system: {error}")))?;
        Self::from_value(&root)
    }

    pub fn from_value(root: &Value) -> Result<Self, Error> {
        known(root, ROOT, "particle system")?;
        let flags = number(root.get("flags")).unwrap_or(0.0);
        if flags < 0.0
            || flags.fract() != 0.0
            || flags as u32 & !(WORLD | CRISP | PERSPECTIVE | FIXED) != 0
        {
            return Err(refuse("particle system flags"));
        }
        let flags = flags as u32;
        // Sprite sheets and child systems; control points matter only to the operators that
        // follow them, which are refused on their own.
        let sheet = match text(root.get("animationmode")) {
            Some("randomframe") => Sheet::Random,
            Some("sequence") | None => Sheet::Sequence(
                number(root.get("sequencemultiplier"))
                    .filter(|times| *times > 0.0 && *times <= 1000.0)
                    .unwrap_or(1.0),
            ),
            Some(_) => return Err(refuse("particle sprite sheet animation")),
        };
        let mut children = Vec::new();
        for child in list(root, "children")? {
            known(child, CHILD, "child system")?;
            // Children let out when particles are born or die aren't drawn yet.
            let follow = match text(child.get("type")) {
                None | Some("static") => false,
                Some("eventfollow") => true,
                Some(_) => return Err(refuse("child particle systems let out by particles")),
            };
            let scale = vector(child.get("scale"), [1.0; 3]);
            let angles = vector(child.get("angles"), [0.0; 3]);
            // One that follows particles sits in its parent's units, moved but not scaled.
            if follow && (scale != [1.0; 3] || angles != [0.0; 3]) {
                return Err(refuse("a scaled or turned child following particles"));
            }
            if number(child.get("probability")).is_some_and(|chance| chance < 1.0) {
                return Err(refuse("child particle systems let out by chance"));
            }
            for key in ["flags", "controlpointstartindex"] {
                if number(child.get(key)).is_some_and(|value| value != 0.0) {
                    return Err(refuse(&format!("child particle system {key}")));
                }
            }
            let file = text(child.get("name"))
                .ok_or_else(|| refuse("a child particle system without a file"))?;
            children.push(Child {
                file: file.to_owned(),
                origin: vector(child.get("origin"), [0.0; 3]),
                scale,
                angles,
                max: number(child.get("maxcount")).map(|max| max.clamp(0.0, MOST as f32) as usize),
                follow,
            });
            if children.len() > 8 {
                return Err(refuse("more than 8 child particle systems"));
            }
        }
        let material = text(root.get("material"))
            .ok_or_else(|| refuse("a particle system without a material"))?
            .to_owned();
        let renderers = list(root, "renderer")?;
        // Stock snow and light shafts leave the renderer out and draw sprites; spawners that
        // draw nothing give an empty list.
        let renderer = match renderers {
            [] if root.get("renderer").is_none_or(Value::is_null) => Renderer::Sprite,
            [renderer] if text(renderer.get("name")) == Some("sprite") => {
                known(renderer, &[], "sprite renderer")?;
                Renderer::Sprite
            }
            [renderer] if text(renderer.get("name")) == Some("spritetrail") => {
                known(
                    renderer,
                    &["length", "maxlength", "minlength"],
                    "trail renderer",
                )?;
                let field = |key: &str, fill: f32| {
                    number(renderer.get(key))
                        .filter(|value| (0.0..=10000.0).contains(value))
                        .unwrap_or(fill)
                };
                let min = field("minlength", 0.0);
                Renderer::Trail {
                    length: field("length", 0.05),
                    min,
                    max: field("maxlength", 10.0).max(min),
                }
            }
            [renderer] if text(renderer.get("name")) == Some("ropetrail") => {
                known(renderer, &["length"], "rope trail renderer")?;
                Renderer::Rope {
                    length: number(renderer.get("length"))
                        .filter(|length| (0.0..=60.0).contains(length))
                        .unwrap_or(1.0),
                }
            }
            _ => return Err(refuse("particle renderers other than one sprite or trail")),
        };
        let emitters = list(root, "emitter")?
            .iter()
            .map(emitter)
            .collect::<Result<Vec<_>, _>>()?;
        if emitters.is_empty() || emitters.len() > 8 {
            return Err(refuse("a particle system with no or many emitters"));
        }
        let mut spawn = Spawn::default();
        for initializer in list(root, "initializer")? {
            initialize(initializer, &mut spawn)?;
        }
        let controls = control::controls(root);
        let operators = list(root, "operator")?
            .iter()
            .map(|value| operator(value, &controls))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(System {
            material,
            max: number(root.get("maxcount"))
                .unwrap_or(100.0)
                .clamp(0.0, MOST as f32) as usize,
            warmup: number(root.get("starttime"))
                .unwrap_or(0.0)
                .clamp(0.0, 30.0),
            emitters,
            spawn,
            operators,
            world: flags & WORLD != 0,
            perspective: flags & PERSPECTIVE != 0,
            crisp: flags & CRISP != 0,
            fixed: flags & FIXED,
            sheet,
            renderer,
            children,
            controls,
        })
    }
}

fn emitter(value: &Value) -> Result<Emitter, Error> {
    let shape = match text(value.get("name")) {
        Some("boxrandom") => Shape::Box,
        Some("sphererandom") => Shape::Sphere,
        other => return Err(refuse(&format!("emitter {}", name(other)))),
    };
    let keys: Vec<&str> = EMITTER.iter().chain(super::audio::KEYS).copied().collect();
    known(value, &keys, "emitter")?;
    let hearing = super::audio::hearing(value, refuse)?;
    let rate = number(value.get("rate")).unwrap_or(5.0);
    // How many are let out stays within the system's count, however fast it asks.
    if !(0.0..=100_000.0).contains(&rate) {
        return Err(refuse("an emitter rate out of range"));
    }
    if number(value.get("flags")).is_some_and(|flags| flags != 0.0) {
        return Err(refuse("emitter flags"));
    }
    if number(value.get("duration")).is_some_and(|duration| duration != 0.0) {
        return Err(refuse("emitters that stop"));
    }
    let sign = vector(value.get("sign"), [0.0; 3]);
    if sign[0] != 0.0 || sign[1] != 0.0 {
        return Err(refuse("emitters signed along x or y"));
    }
    if shape == Shape::Box && (value.get("speedmin").is_some() || value.get("speedmax").is_some()) {
        return Err(refuse("box emitters with a speed"));
    }
    let burst = number(value.get("instantaneous")).unwrap_or(0.0);
    if !(0.0..=10_000.0).contains(&burst) {
        return Err(refuse("an emitter burst out of range"));
    }
    let bound = |key: &str, fill: f32| match shape {
        Shape::Box => vector(value.get(key), [fill; 3]),
        Shape::Sphere => [number(value.get(key)).unwrap_or(fill); 3],
    };
    Ok(Emitter {
        shape,
        rate,
        origin: vector(value.get("origin"), [0.0; 3]),
        min: bound("distancemin", 0.0),
        max: bound("distancemax", 256.0),
        directions: vector(value.get("directions"), [1.0, 1.0, 0.0]),
        speed: [
            number(value.get("speedmin")).unwrap_or(0.0),
            number(value.get("speedmax")).unwrap_or(0.0),
        ],
        burst: burst as usize,
        hearing,
    })
}

fn initialize(value: &Value, spawn: &mut Spawn) -> Result<(), Error> {
    let kind = text(value.get("name"));
    if kind == Some("turbulentvelocityrandom") {
        known(value, TURBULENT, "initializer")?;
        let field = |key: &str, fill: f32| number(value.get(key)).unwrap_or(fill);
        let low = field("speedmin", 100.0);
        spawn.turbulent = Some(Turbulent {
            forward: vector(value.get("forward"), [0.0, 1.0, 0.0]),
            offset: field("offset", 0.0),
            scale: field("scale", 1.0),
            speed: [low, field("speedmax", 250.0).max(low)],
            phase: ordered(field("phasemin", 0.0), field("phasemax", 0.1)),
            timescale: field("timescale", 1.0),
        });
        return Ok(());
    }
    known(value, RANGE, "initializer")?;
    let exponent = number(value.get("exponent")).unwrap_or(1.0);
    if !(0.01..=100.0).contains(&exponent) {
        return Err(refuse("an initializer exponent out of range"));
    }
    let scalar = |fill: f32| Between {
        low: number(value.get("min")).unwrap_or(fill),
        high: number(value.get("max")).unwrap_or(fill),
        exponent,
    };
    let triple = |fill: [f32; 3]| Between {
        low: vector(value.get("min"), fill),
        high: vector(value.get("max"), fill),
        exponent,
    };
    match kind {
        Some("lifetimerandom") => spawn.life = scalar(1.0),
        Some("sizerandom") => spawn.size = scalar(20.0),
        Some("alpharandom") => spawn.alpha = scalar(1.0),
        // Colours here are written in 0 to 255.
        Some("colorrandom") => {
            let color = triple([255.0; 3]);
            spawn.color = Between {
                low: color.low.map(|c| c / 255.0),
                high: color.high.map(|c| c / 255.0),
                exponent,
            }
        }
        Some("velocityrandom") => spawn.velocity = triple([0.0; 3]),
        Some("rotationrandom") => spawn.rotation = triple([0.0; 3]),
        Some("angularvelocityrandom") => spawn.spin = triple([0.0; 3]),
        other => return Err(refuse(&format!("initializer {}", name(other)))),
    }
    if [spawn.life.low, spawn.life.high]
        .iter()
        .any(|life| !(0.001..=600.0).contains(life))
    {
        return Err(refuse("a particle lifetime out of range"));
    }
    Ok(())
}

const OSCILLATE: &[&str] = &[
    "blendinend",
    "blendinstart",
    "blendoutend",
    "blendoutstart",
    "frequencymax",
    "frequencymin",
    "mask",
    "phasemax",
    "phasemin",
    "scalemax",
    "scalemin",
];

fn operator(value: &Value, controls: &[Control; CONTROLS]) -> Result<Operator, Error> {
    let field = |key: &str, fill: f32| number(value.get(key)).unwrap_or(fill);
    let times = || [field("starttime", 0.0), field("endtime", 1.0)];
    let operator = match text(value.get("name")) {
        Some("movement") => {
            known(value, &["drag", "gravity"], "operator")?;
            Operator::Movement {
                gravity: vector(value.get("gravity"), [0.0; 3]),
                drag: field("drag", 0.0),
            }
        }
        Some("angularmovement") => {
            known(value, &["drag", "force"], "operator")?;
            Operator::AngularMovement {
                force: vector(value.get("force"), [0.0; 3]),
                drag: field("drag", 0.0),
            }
        }
        Some("alphafade") => {
            known(value, &["fadeintime", "fadeouttime"], "operator")?;
            Operator::AlphaFade {
                fade_in: field("fadeintime", 0.5).clamp(0.0, 1.0),
                fade_out: field("fadeouttime", 0.5).clamp(0.0, 1.0),
            }
        }
        Some("sizechange") => {
            known(value, CHANGE, "operator")?;
            Operator::SizeChange {
                times: times(),
                values: [field("startvalue", 1.0), field("endvalue", 0.0)],
            }
        }
        Some("alphachange") => {
            known(value, CHANGE, "operator")?;
            Operator::AlphaChange {
                times: times(),
                values: [field("startvalue", 1.0), field("endvalue", 0.0)],
            }
        }
        Some("colorchange") => {
            known(value, CHANGE, "operator")?;
            Operator::ColorChange {
                times: times(),
                values: [
                    vector(value.get("startvalue"), [1.0; 3]),
                    vector(value.get("endvalue"), [1.0; 3]),
                ],
            }
        }
        Some(kind @ ("oscillatealpha" | "oscillatesize" | "oscillateposition")) => {
            known(value, OSCILLATE, "operator")?;
            let what = match kind {
                "oscillatealpha" => Swing::Alpha,
                "oscillatesize" => Swing::Size,
                _ => Swing::Place,
            };
            let range = |low: &str, high: &str, fill: [f32; 2]| {
                let low = field(low, fill[0]);
                [low, field(high, fill[1]).max(low)]
            };
            Operator::Oscillate {
                what,
                frequency: range("frequencymin", "frequencymax", [0.0, 10.0]),
                scale: range("scalemin", "scalemax", [0.0, 1.0]),
                phase: range("phasemin", "phasemax", [0.0, std::f32::consts::TAU]),
                mask: vector(value.get("mask"), [1.0, 1.0, 0.0]),
                blend: [
                    field("blendinstart", 0.0),
                    field("blendinend", 0.0),
                    field("blendoutstart", 1.0),
                    field("blendoutend", 1.0),
                ],
            }
        }
        Some("controlpointattract") => {
            known(value, ATTRACT, "operator")?;
            if field("flags", 0.0) != 0.0 {
                return Err(refuse("control point attraction flags"));
            }
            Operator::Attract {
                control: control::named(value, controls, refuse)?,
                origin: vector(value.get("origin"), [0.0; 3]),
                scale: field("scale", 0.0),
                threshold: field("threshold", 128.0).max(0.0),
                blend: [
                    field("blendinstart", 0.0),
                    field("blendinend", 0.0),
                    field("blendoutstart", 1.0),
                    field("blendoutend", 1.0),
                ],
            }
        }
        Some("turbulence") => {
            known(value, TURBULENCE, "operator")?;
            let low = field("speedmin", 500.0);
            Operator::Turbulence {
                scale: field("scale", 0.01),
                timescale: field("timescale", 1.0),
                speed: [low, field("speedmax", 1000.0).max(low)],
                phase: ordered(field("phasemin", 0.0), field("phasemax", 0.1)),
                mask: vector(value.get("mask"), [1.0, 1.0, 0.0]),
                blend: [
                    field("blendinstart", 0.0),
                    field("blendinend", 0.0),
                    field("blendoutstart", 1.0),
                    field("blendoutend", 1.0),
                ],
            }
        }
        other => return Err(refuse(&format!("operator {}", name(other)))),
    };
    Ok(operator)
}

fn ordered(low: f32, high: f32) -> [f32; 2] {
    [low.min(high), high.max(low)]
}

fn list<'a>(root: &'a Value, key: &str) -> Result<&'a [Value], Error> {
    match root.get(key) {
        None | Some(Value::Null) => Ok(&[]),
        Some(Value::Array(items)) if items.len() <= 32 => Ok(items),
        Some(_) => Err(refuse(&format!("particle {key} that isn't a short list"))),
    }
}

/// Refuses keys other than `name`, `id` and those given.
fn known(value: &Value, keys: &[&str], what: &str) -> Result<(), Error> {
    let fields = value
        .as_object()
        .ok_or_else(|| refuse(&format!("a {what} that isn't an object")))?;
    for key in fields.keys() {
        if key != "name" && key != "id" && !keys.contains(&key.as_str()) {
            let key: String = key.chars().take(64).collect();
            return Err(refuse(&format!("{what} field {key}")));
        }
    }
    let changing = fields.values().any(|value| {
        value
            .as_object()
            .is_some_and(|fields| fields.keys().any(|key| key != "value" && key != "user"))
    });
    if changing {
        return Err(refuse(&format!("an animated {what} value")));
    }
    Ok(())
}

fn name(kind: Option<&str>) -> String {
    kind.unwrap_or("without a name").chars().take(64).collect()
}

fn refuse(what: &str) -> Error {
    Error::Format(format!("unsupported {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAIN: &str = r#"{
        "material": "materials/particle/drop.json", "maxcount": 300, "starttime": 2,
        "emitter": [{"name": "boxrandom", "id": 1, "rate": 40, "origin": "960 1200 0",
                     "distancemax": "960 10 0", "directions": "1 1 0"}],
        "initializer": [{"name": "lifetimerandom", "min": 2, "max": 3},
                        {"name": "colorrandom", "min": "255 255 255", "max": "128 128 255"},
                        {"name": "velocityrandom", "min": "-10 -400 0", "max": "10 -300 0"}],
        "operator": [{"name": "movement", "gravity": "0 -50 0"},
                     {"name": "alphafade", "fadeintime": 0.1, "fadeouttime": 0.8}],
        "renderer": [{"name": "sprite"}]
    }"#;

    #[test]
    fn a_system_reads_with_its_parts_in_order() {
        let system = System::parse(RAIN).unwrap();
        assert_eq!((system.max, system.warmup), (300, 2.0));
        assert_eq!(system.emitters[0].shape, Shape::Box);
        assert_eq!(system.emitters[0].max, [960.0, 10.0, 0.0]);
        assert_eq!((system.spawn.life.low, system.spawn.life.high), (2.0, 3.0));
        assert_eq!(system.spawn.color.high, [128.0 / 255.0, 128.0 / 255.0, 1.0]);
        assert_eq!(system.operators.len(), 2);
        assert!(!system.world && !system.perspective);
        // Worldspace is 1 and perspective rendering 4.
        let flagged = |flags: u32| {
            System::parse(&RAIN.replace(
                r#""starttime": 2"#,
                &format!(r#""starttime": 2, "flags": {flags}"#),
            ))
        };
        // A rope trail reads its length; a fading one isn't drawn yet.
        let roped =
            |renderer: &str| System::parse(&RAIN.replace(r#"[{"name": "sprite"}]"#, renderer));
        assert_eq!(
            roped(r#"[{"name": "ropetrail", "length": 0.2}]"#)
                .unwrap()
                .renderer,
            Renderer::Rope { length: 0.2 }
        );
        assert!(roped(r#"[{"name": "ropetrail", "length": 0.5, "fadealpha": true}]"#).is_err());
        // Without a renderer, it's drawn as sprites.
        let unnamed = RAIN.replace(
            r#",
        "renderer": [{"name": "sprite"}]"#,
            "",
        );
        assert!(!unnamed.contains("renderer"));
        assert_eq!(System::parse(&unnamed).unwrap().renderer, Renderer::Sprite);
        let both = flagged(5).unwrap();
        assert!(both.world && both.perspective && !both.crisp);
        // 2 shows sprite sheet frames whole; 8 to 128 leave colour, count, lifetime, size and
        // speed overrides out.
        let fixed = flagged(2 | 16 | 128).unwrap();
        assert!(fixed.crisp);
        assert!(!fixed.overridable("count") && !fixed.overridable("speed"));
        assert!(fixed.overridable("colorn") && fixed.overridable("rate"));
        assert!(
            flagged(256)
                .err()
                .unwrap()
                .to_string()
                .contains("particle system flags")
        );
    }

    #[test]
    fn anything_not_understood_is_refused_by_name() {
        for (from, to, expected) in [
            (
                r#""name": "sprite""#,
                r#""name": "rope""#,
                "renderers other",
            ),
            (
                r#""name": "boxrandom""#,
                r#""name": "boxrandom", "audioprocessingmode": 1"#,
                "audio response on one channel",
            ),
            (
                r#""renderer": [{"name": "sprite"}]"#,
                r#""renderer": []"#,
                "renderers other",
            ),
            (
                r#""name": "boxrandom""#,
                r#""name": "boxrandom", "flags": 2"#,
                "emitter flags",
            ),
            (
                r#""starttime": 2"#,
                r#""starttime": 2, "animationmode": "pingpong""#,
                "sprite sheet",
            ),
            (
                r#""starttime": 2"#,
                r#""starttime": 2, "children": [{"name": "x", "type": "eventdeath"}]"#,
                "child particle",
            ),
            (
                r#""starttime": 2"#,
                r#""starttime": 2, "children": [{"name": "x", "controlpointstartindex": 1}]"#,
                "child particle system controlpointstartindex",
            ),
            (
                r#""name": "movement""#,
                r#""name": "vortex""#,
                "operator vortex",
            ),
            (
                r#""starttime": 2"#,
                r#""starttime": 2, "controlpoints": []"#,
                "field controlpoints",
            ),
            (
                r#""rate": 40"#,
                r#""rate": {"value": 40, "animation": {}}"#,
                "animated emitter",
            ),
            (
                r#""min": 2, "max": 3"#,
                r#""min": 0, "max": 3"#,
                "lifetime out of range",
            ),
        ] {
            let json = RAIN.replacen(from, to, 1);
            let error = System::parse(&json).unwrap_err().to_string();
            assert!(error.contains(expected), "{expected}: {error}");
        }
    }
}
