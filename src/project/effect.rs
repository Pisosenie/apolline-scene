//! Effects on a layer. Each instance names an `effect.json`, whose passes name materials, and
//! its own passes override their combos, values and textures. Every pass draws over the
//! layer's picture so far, which it reads in texture slot 0. What isn't understood — render
//! targets, passes that blend, values that change — is refused by name.

use super::material::{Resources, names, object};
use super::*;
use crate::shader::Fragment;

/// One pass of an effect, drawn over the layer's picture so far.
#[derive(Clone)]
pub struct EffectPass {
    pub fragment: Arc<Fragment>,
    /// Textures in slots 1 to 7.
    pub inputs: Vec<TextureInput>,
}

const INSTANCE: &[&str] = &["file", "id", "name", "passes", "visible"];
const DEFINITION: &[&str] = &[
    "dependencies",
    "description",
    "group",
    "name",
    "passes",
    "preview",
    "version",
];
const STEP: &[&str] = &["material"];
const OVERRIDE: &[&str] = &["combos", "constantshadervalues", "id", "textures"];
const PASS: &[&str] = &[
    "blending",
    "combos",
    "constantshadervalues",
    "cullmode",
    "depthtest",
    "depthwrite",
    "shader",
    "textures",
];

impl Project {
    /// The passes of an image's shown effects, in drawing order.
    pub(super) fn effects(
        &self,
        image: &Image,
        resources: &mut Resources,
        remaining: &mut usize,
        still: bool,
    ) -> Result<Vec<EffectPass>, Error> {
        let mut passes = Vec::new();
        for instance in &image.effects {
            let file = text(instance.get("file")).unwrap_or_default();
            self.effect(instance, file, resources, remaining, still, &mut passes)
                .map_err(|error| Error::Format(format!("effect {}: {error}", name(file))))?;
            if passes.len() > 32 {
                return Err(Error::Format(
                    "a layer with more than 32 effect passes".into(),
                ));
            }
        }
        Ok(passes)
    }

    fn effect(
        &self,
        instance: &Value,
        file: &str,
        resources: &mut Resources,
        remaining: &mut usize,
        still: bool,
        passes: &mut Vec<EffectPass>,
    ) -> Result<(), Error> {
        known(instance, INSTANCE)?;
        if !relative(file) {
            return Err(Error::Format(
                "names no effect file inside the project".into(),
            ));
        }
        let definition = self.json(file)?;
        if scripted(&definition) {
            return Err(Error::Format(
                "a scripted effect needs another renderer".into(),
            ));
        }
        known(&definition, DEFINITION)?;
        let steps = definition
            .get("passes")
            .and_then(Value::as_array)
            .filter(|steps| !steps.is_empty() && steps.len() <= 8)
            .ok_or_else(|| Error::Format("effect.json needs one to eight passes".into()))?;
        let overrides = match instance.get("passes") {
            None => &[][..],
            Some(Value::Array(overrides)) if overrides.len() <= steps.len() => overrides,
            Some(_) => {
                return Err(Error::Format("its passes don't match the effect's".into()));
            }
        };
        for (at, step) in steps.iter().enumerate() {
            known(step, STEP)?;
            let material = text(step.get("material"))
                .ok_or_else(|| Error::Format("an effect pass names no material".into()))?;
            let over = overrides.get(at);
            if let Some(over) = over {
                known(over, OVERRIDE)?;
            }
            passes.push(self.effect_pass(material, over, resources, remaining, still)?);
        }
        Ok(())
    }

    fn effect_pass(
        &self,
        material: &str,
        over: Option<&Value>,
        resources: &mut Resources,
        remaining: &mut usize,
        still: bool,
    ) -> Result<EffectPass, Error> {
        if !relative(material) {
            return Err(Error::Format(
                "material path must stay within its resource root".into(),
            ));
        }
        let json = self.json(material)?;
        if scripted(&json) {
            return Err(Error::Format(
                "a scripted material needs another renderer".into(),
            ));
        }
        let pass = match json
            .get("passes")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
        {
            Some([pass]) => pass,
            _ => return Err(Error::Format("an effect material needs one pass".into())),
        };
        known(pass, PASS)?;
        if !matches!(
            pass.get("blending").and_then(|value| text(Some(value))),
            None | Some("normal" | "disabled")
        ) {
            return Err(Error::Format(
                "effect passes that blend need another renderer".into(),
            ));
        }
        let shader = text(pass.get("shader"))
            .ok_or_else(|| Error::Format("an effect material names no shader".into()))?;
        let mut combos = object(pass.get("combos"), 64, 64)?;
        let mut constants = object(pass.get("constantshadervalues"), 128, 128)?;
        let mut textures = names(pass.get("textures"))?;
        if textures.first().is_some_and(Option::is_some) {
            return Err(Error::Format(
                "an effect material binds a texture over the layer's picture".into(),
            ));
        }
        if let Some(over) = over {
            combos.extend(object(over.get("combos"), 64, 64)?);
            constants.extend(object(over.get("constantshadervalues"), 128, 128)?);
            for (slot, name) in names(over.get("textures"))?.into_iter().enumerate() {
                if let Some(name) = name {
                    if slot == 0 {
                        return Err(Error::Format(
                            "an effect binds a texture over the layer's picture".into(),
                        ));
                    }
                    if textures.len() <= slot {
                        textures.resize(slot + 1, None);
                    }
                    textures[slot] = Some(name);
                }
            }
        }
        if combos.len() > 64 || constants.len() > 128 {
            return Err(Error::Format("too many effect combos or values".into()));
        }
        if combos.values().chain(constants.values()).any(changing) {
            return Err(Error::Format(
                "animated effect values need another renderer".into(),
            ));
        }
        let fragment = self.custom(shader, &combos, &constants, &mut textures, true, resources)?;
        let mut inputs = Vec::new();
        for (slot, name) in textures.iter().enumerate().skip(1) {
            if let Some(name) = name {
                inputs.push(TextureInput {
                    slot: slot as u8,
                    texture: self.texture(name, resources, remaining, still)?,
                });
            }
        }
        Ok(EffectPass { fragment, inputs })
    }
}

/// Refuses a key that isn't among those understood.
fn known(value: &Value, keys: &[&str]) -> Result<(), Error> {
    let fields = value
        .as_object()
        .ok_or_else(|| Error::Format("an effect entry that isn't an object".into()))?;
    match fields.keys().find(|key| !keys.contains(&key.as_str())) {
        Some(key) => Err(Error::Format(format!(
            "unsupported field {}",
            key.chars().take(64).collect::<String>()
        ))),
        None => Ok(()),
    }
}

/// A value with more in it than a property binding: an animation or a script.
fn changing(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|fields| fields.keys().any(|key| key != "value" && key != "user"))
}

/// `effects/waterripple/effect.json` → `waterripple`.
fn name(file: &str) -> String {
    file.strip_prefix("effects/")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(file)
        .chars()
        .take(64)
        .collect()
}
