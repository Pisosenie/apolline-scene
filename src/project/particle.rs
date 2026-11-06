//! A particle system in the scene: its file, and the material its sprites are drawn with,
//! which has to be Wallpaper Engine's plain particle shader with one texture.

use super::material::{Resources, names, object};
use super::*;
use crate::particles::{Child, System};
use crate::scene::Particles;
use crate::value::number;

/// How deep children of children may go.
const GENERATIONS: usize = 3;

impl Project {
    /// The system's layer, then its children's, each drawn after the one it comes from.
    pub(super) fn particle_layers(
        &self,
        placed: &Particles,
        resources: &mut Resources,
        remaining: &mut usize,
    ) -> Result<Vec<Layer>, Error> {
        let mut family = vec![self.particle_layer(placed, None, resources, remaining)?];
        // How many systems each one is down from the top.
        let mut generations = vec![0];
        let mut at = 0;
        while let Some(parent) = family.get(at) {
            let children = parent
                .particles
                .as_ref()
                .map_or(&[][..], |system| &system.children[..]);
            if !children.is_empty() && generations[at] >= GENERATIONS {
                return Err(Error::Format(format!(
                    "particle systems more than {GENERATIONS} generations deep"
                )));
            }
            let mut born = Vec::new();
            for child in children {
                let file = Particles {
                    file: child.file.clone(),
                    ..placed.clone()
                };
                let layer = self.particle_layer(&file, child.max, resources, remaining)?;
                // The family's layers are drawn in this order, so its parent is this far back.
                let behind = family.len() + born.len() - at;
                born.push(Layer {
                    child: Some(within(parent.child.as_ref(), child)),
                    leader: child.follow.then_some(behind),
                    ..layer
                });
            }
            let generation = generations[at] + 1;
            generations.extend(born.iter().map(|_| generation));
            family.extend(born);
            if family.len() > 64 {
                return Err(Error::Format("more than 64 particle systems in one".into()));
            }
            at += 1;
        }
        Ok(family)
    }

    /// One system's layer; `max`, a child's own count, takes the place of its file's.
    fn particle_layer(
        &self,
        placed: &Particles,
        max: Option<usize>,
        resources: &mut Resources,
        remaining: &mut usize,
    ) -> Result<Layer, Error> {
        if !relative(&placed.file) {
            return Err(Error::Format(
                "particle file must stay inside the project".into(),
            ));
        }
        let json = self.json(&placed.file)?;
        if scripted(&json) {
            return Err(Error::Format(
                "scripted particles need another renderer".into(),
            ));
        }
        let system = System::from_value(&json)?;
        let system = System {
            max: max.unwrap_or(system.max),
            ..system
        };
        let system = overridden(system, &placed.overrides)?;
        if !relative(&system.material) {
            return Err(Error::Format(
                "particle material must stay inside the project".into(),
            ));
        }
        let material = self.json(&system.material)?;
        let pass = match material
            .get("passes")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
        {
            Some([pass]) => pass,
            _ => return Err(Error::Format("a particle material needs one pass".into())),
        };
        if let Some(key) = pass.as_object().and_then(|pass| {
            pass.keys().find(|key| {
                !matches!(
                    key.as_str(),
                    "shader"
                        | "textures"
                        | "blending"
                        | "combos"
                        | "constantshadervalues"
                        | "cullmode"
                        | "depthtest"
                        | "depthwrite"
                )
            })
        }) {
            let key: String = key.chars().take(64).collect();
            return Err(Error::Format(format!(
                "unsupported particle material field {key}"
            )));
        }
        if !matches!(text(pass.get("shader")), None | Some("genericparticle")) {
            return Err(Error::Format(
                "particles with their own shader need another renderer".into(),
            ));
        }
        let combos = object(pass.get("combos"), 64, 64)?;
        let values = object(pass.get("constantshadervalues"), 128, 128)?;
        // Overbright brightens the sprites; any other value changes what the shader does.
        let overbright = values.get("ui_editor_properties_overbright");
        if combos
            .values()
            .any(|value| number(Some(value)).is_none_or(|value| value != 0.0))
            || values.len() > usize::from(overbright.is_some())
        {
            return Err(Error::Format(
                "particle shader options need another renderer".into(),
            ));
        }
        let overbright = match overbright {
            None => 1.0,
            Some(value) => number(Some(value))
                .filter(|value| (0.0..=100.0).contains(value))
                .ok_or_else(|| Error::Format("a particle overbright that isn't a number".into()))?,
        };
        let blending = match text(pass.get("blending")) {
            None | Some("translucent") => Blending::Translucent,
            Some("additive") => Blending::Additive,
            Some("normal" | "disabled") => Blending::Opaque,
            Some(_) => return Err(Error::Format("unsupported particle blending".into())),
        };
        let texture = match names(pass.get("textures"))?.as_slice() {
            [Some(name)] => self.texture(name, resources, remaining, false)?,
            _ => {
                return Err(Error::Format(
                    "a particle material needs one texture".into(),
                ));
            }
        };
        // A sprite sheet's frames, one image's and all alike, which particles move through.
        if texture.animated() {
            let first = texture.frames.first();
            let alike = first.is_some_and(|first| {
                texture.images.len() == 1
                    && texture
                        .frames
                        .iter()
                        .all(|frame| frame.across == first.across && frame.down == first.down)
            });
            if !alike {
                return Err(Error::Format(
                    "animated particle sprites need another renderer".into(),
                ));
            }
        }
        Ok(Layer {
            object: 0,
            shade: [overbright, 1.0],
            image: Image {
                name: placed.name.clone(),
                model: placed.file.clone(),
                origin: placed.origin,
                scale: placed.scale,
                angles: placed.angles,
                size: None,
                color: [1.0; 3],
                alpha: 1.0,
                brightness: 1.0,
                anchor: [0.0; 2],
                visible: placed.visible,
                depth: placed.depth,
                effects: Vec::new(),
                instance: None,
                mode: 0,
                animations: Vec::new(),
            },
            texture,
            secondary: Vec::new(),
            fragment: None,
            blending,
            fullscreen: false,
            effects: Vec::new(),
            undrawn: Vec::new(),
            sprite: false,
            compose: false,
            particles: Some(Arc::new(system)),
            child: None,
            leader: None,
            puppet: None,
            text: None,
        })
    }
}

/// A child's place from the top system's, through the one it comes from.
fn within(parent: Option<&Child>, child: &Child) -> Child {
    let Some(parent) = parent else {
        return child.clone();
    };
    let (sin, cos) = parent.angles[2].sin_cos();
    let [x, y, z] = [
        child.origin[0] * parent.scale[0],
        child.origin[1] * parent.scale[1],
        child.origin[2] * parent.scale[2],
    ];
    Child {
        file: child.file.clone(),
        origin: [
            parent.origin[0] + cos * x - sin * y,
            parent.origin[1] + sin * x + cos * y,
            parent.origin[2] + z,
        ],
        scale: [0, 1, 2].map(|axis| parent.scale[axis] * child.scale[axis]),
        angles: [0, 1, 2].map(|axis| parent.angles[axis] + child.angles[axis]),
        max: child.max,
        follow: child.follow,
    }
}

/// The system as this instance changes it: each override scales what it names.
fn overridden(
    mut system: System,
    overrides: &serde_json::Map<String, Value>,
) -> Result<System, Error> {
    for (key, value) in overrides {
        // Scripted ones scale the system as it runs, from where its file leaves it; the older
        // byte colour gives way to the newer one.
        let scripted = value.get("script").is_some();
        let older = key == "color" && overrides.contains_key("colorn");
        if key == "id" || scripted || older || !system.overridable(key) {
            continue;
        }
        let factor = || {
            number(Some(value))
                .filter(|factor| factor.is_finite() && (0.0..=100.0).contains(factor))
                .ok_or_else(|| Error::Format(format!("particle override {key} out of range")))
        };
        match key.as_str() {
            "count" => {
                let most = crate::particles::MOST as f32;
                system.max = ((system.max as f32) * factor()?).min(most) as usize;
            }
            "rate" => {
                let factor = factor()?;
                for emitter in &mut system.emitters {
                    emitter.rate *= factor;
                }
            }
            "lifetime" => scale(&mut system.spawn.life, factor()?),
            "size" => scale(&mut system.spawn.size, factor()?),
            "alpha" => scale(&mut system.spawn.alpha, factor()?),
            "speed" => {
                let factor = factor()?;
                let velocity = &mut system.spawn.velocity;
                velocity.low = velocity.low.map(|v| v * factor);
                velocity.high = velocity.high.map(|v| v * factor);
                for emitter in &mut system.emitters {
                    emitter.speed = emitter.speed.map(|v| v * factor);
                }
                if let Some(turbulent) = &mut system.spawn.turbulent {
                    turbulent.speed = turbulent.speed.map(|v| v * factor);
                }
            }
            // A tint, from 0 to 1 or, in older scenes, 0 to 255.
            "colorn" | "color" => {
                let bytes = if key == "color" { 255.0 } else { 1.0 };
                let tint = crate::value::vector(Some(value), [bytes; 3]).map(|tint| tint / bytes);
                let color = &mut system.spawn.color;
                for (channel, tint) in tint.iter().enumerate() {
                    color.low[channel] *= tint;
                    color.high[channel] *= tint;
                }
            }
            _ => {
                let key: String = key.chars().take(32).collect();
                return Err(Error::Format(format!(
                    "unsupported particle override {key}"
                )));
            }
        }
    }
    if system.spawn.life.low < 0.001 || system.spawn.life.high < 0.001 {
        return Err(Error::Format("particle override leaves no lifetime".into()));
    }
    Ok(system)
}

fn scale(range: &mut crate::particles::Between<f32>, factor: f32) {
    range.low *= factor;
    range.high *= factor;
}
