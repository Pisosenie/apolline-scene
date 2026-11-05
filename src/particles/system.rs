use serde_json::Value;

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
}

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
}

/// What a new particle starts with, each drawn between a lower and an upper bound.
#[derive(Clone, Debug, PartialEq)]
pub struct Spawn {
    pub life: [f32; 2],
    pub size: [f32; 2],
    pub alpha: [f32; 2],
    pub color: [[f32; 3]; 2],
    pub velocity: [[f32; 3]; 2],
    pub rotation: [[f32; 3]; 2],
    pub spin: [[f32; 3]; 2],
}

impl Default for Spawn {
    fn default() -> Self {
        Spawn {
            life: [1.0; 2],
            size: [20.0; 2],
            alpha: [1.0; 2],
            color: [[1.0; 3]; 2],
            velocity: [[0.0; 3]; 2],
            rotation: [[0.0; 3]; 2],
            spin: [[0.0; 3]; 2],
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
}

const ROOT: &[&str] = &[
    "emitter",
    "flags",
    "id",
    "initializer",
    "material",
    "maxcount",
    "name",
    "operator",
    "renderer",
    "starttime",
];
const EMITTER: &[&str] = &["directions", "distancemax", "distancemin", "origin", "rate"];
const RANGE: &[&str] = &["max", "min"];
const CHANGE: &[&str] = &["endtime", "endvalue", "starttime", "startvalue"];

impl System {
    pub fn parse(json: &str) -> Result<Self, Error> {
        let root: Value = serde_json::from_str(json)
            .map_err(|error| Error::Format(format!("particle system: {error}")))?;
        Self::from_value(&root)
    }

    pub fn from_value(root: &Value) -> Result<Self, Error> {
        known(root, ROOT, "particle system")?;
        if number(root.get("flags")).is_some_and(|flags| flags != 0.0) {
            return Err(refuse("particle system flags"));
        }
        let material = text(root.get("material"))
            .ok_or_else(|| refuse("a particle system without a material"))?
            .to_owned();
        let renderers = list(root, "renderer")?;
        match renderers {
            [renderer] if text(renderer.get("name")) == Some("sprite") => {
                known(renderer, &[], "sprite renderer")?
            }
            _ => return Err(refuse("particle renderers other than one sprite")),
        }
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
        let operators = list(root, "operator")?
            .iter()
            .map(operator)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(System {
            material,
            max: number(root.get("maxcount"))
                .unwrap_or(100.0)
                .clamp(0.0, 10_000.0) as usize,
            warmup: number(root.get("starttime"))
                .unwrap_or(0.0)
                .clamp(0.0, 30.0),
            emitters,
            spawn,
            operators,
        })
    }
}

fn emitter(value: &Value) -> Result<Emitter, Error> {
    let shape = match text(value.get("name")) {
        Some("boxrandom") => Shape::Box,
        Some("sphererandom") => Shape::Sphere,
        other => return Err(refuse(&format!("emitter {}", name(other)))),
    };
    known(value, EMITTER, "emitter")?;
    let rate = number(value.get("rate")).unwrap_or(5.0);
    if !(0.0..=10_000.0).contains(&rate) {
        return Err(refuse("an emitter rate out of range"));
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
    })
}

fn initialize(value: &Value, spawn: &mut Spawn) -> Result<(), Error> {
    let kind = text(value.get("name"));
    known(value, RANGE, "initializer")?;
    let scalar = |fill: [f32; 2]| {
        [
            number(value.get("min")).unwrap_or(fill[0]),
            number(value.get("max")).unwrap_or(fill[1]),
        ]
    };
    let triple = |fill: [[f32; 3]; 2]| {
        [
            vector(value.get("min"), fill[0]),
            vector(value.get("max"), fill[1]),
        ]
    };
    match kind {
        Some("lifetimerandom") => spawn.life = scalar([1.0, 1.0]),
        Some("sizerandom") => spawn.size = scalar([20.0, 20.0]),
        Some("alpharandom") => spawn.alpha = scalar([1.0, 1.0]),
        // Colours here are written in 0 to 255.
        Some("colorrandom") => {
            spawn.color = triple([[255.0; 3]; 2]).map(|color| color.map(|c| c / 255.0))
        }
        Some("velocityrandom") => spawn.velocity = triple([[0.0; 3]; 2]),
        Some("rotationrandom") => spawn.rotation = triple([[0.0; 3]; 2]),
        Some("angularvelocityrandom") => spawn.spin = triple([[0.0; 3]; 2]),
        other => return Err(refuse(&format!("initializer {}", name(other)))),
    }
    if spawn
        .life
        .iter()
        .any(|life| !(0.001..=600.0).contains(life))
    {
        return Err(refuse("a particle lifetime out of range"));
    }
    Ok(())
}

fn operator(value: &Value) -> Result<Operator, Error> {
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
        other => return Err(refuse(&format!("operator {}", name(other)))),
    };
    Ok(operator)
}

fn list<'a>(root: &'a Value, key: &str) -> Result<&'a [Value], Error> {
    match root.get(key) {
        None => Ok(&[]),
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
        assert_eq!(system.spawn.life, [2.0, 3.0]);
        assert_eq!(system.spawn.color[1], [128.0 / 255.0, 128.0 / 255.0, 1.0]);
        assert_eq!(system.operators.len(), 2);
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
                r#""name": "boxrandom", "instantaneous": 5"#,
                "emitter field instantaneous",
            ),
            (
                r#""name": "movement""#,
                r#""name": "turbulence""#,
                "operator turbulence",
            ),
            (
                r#""starttime": 2"#,
                r#""starttime": 2, "controlpoint": []"#,
                "field controlpoint",
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
