//! A particle system in the scene: its file, and the material its sprites are drawn with,
//! which has to be Wallpaper Engine's plain particle shader with one texture.

use super::material::{Resources, names, object};
use super::*;
use crate::particles::System;
use crate::scene::Particles;
use crate::value::number;

impl Project {
    pub(super) fn particle_layer(
        &self,
        placed: &Particles,
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
        let system = overridden(System::from_value(&json)?, &placed.overrides)?;
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
            },
            texture,
            secondary: Vec::new(),
            fragment: None,
            blending,
            fullscreen: false,
            effects: Vec::new(),
            undrawn: Vec::new(),
            sprite: false,
            particles: Some(Arc::new(system)),
            text: None,
        })
    }
}

/// The system as this instance changes it: each override scales what it names.
fn overridden(
    mut system: System,
    overrides: &serde_json::Map<String, Value>,
) -> Result<System, Error> {
    for (key, value) in overrides {
        if key == "id" {
            continue;
        }
        let factor = || {
            number(Some(value))
                .filter(|factor| factor.is_finite() && (0.0..=100.0).contains(factor))
                .ok_or_else(|| Error::Format(format!("particle override {key} out of range")))
        };
        match key.as_str() {
            "count" => system.max = ((system.max as f32) * factor()?).min(10_000.0) as usize,
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
            }
            "colorn" => {
                let tint = crate::value::vector(Some(value), [1.0; 3]);
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
