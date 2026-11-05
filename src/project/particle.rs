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
        let system = System::from_value(&json)?;
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
        if combos
            .values()
            .any(|value| number(Some(value)).is_none_or(|value| value != 0.0))
            || !object(pass.get("constantshadervalues"), 128, 128)?.is_empty()
        {
            return Err(Error::Format(
                "particle shader options need another renderer".into(),
            ));
        }
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
        if texture.animated() {
            return Err(Error::Format(
                "animated particle sprites need another renderer".into(),
            ));
        }
        Ok(Layer {
            image: Image {
                name: placed.name.clone(),
                model: placed.file.clone(),
                origin: placed.origin,
                scale: placed.scale,
                angles: placed.angles,
                size: None,
                color: [1.0; 3],
                alpha: 1.0,
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
            particles: Some(Arc::new(system)),
        })
    }
}
