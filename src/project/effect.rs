//! Effects on a layer. Each instance names an `effect.json`, whose passes name materials, and
//! its own passes override their combos, values and textures. Every pass draws over the
//! layer's picture so far, which it reads in texture slot 0. Scripts may show and hide effects
//! and change their values as the scene runs. What isn't understood — render targets, passes
//! that blend, animated values — is refused by name.

use super::material::{Resources, names, object};
use super::*;
use crate::shader::Fragment;

/// One pass of an effect, drawn over the layer's picture so far.
#[derive(Clone)]
pub struct EffectPass {
    pub fragment: Arc<Fragment>,
    /// Textures in slots 1 to 7.
    pub inputs: Vec<TextureInput>,
    /// The effect's place in the layer's list and the pass's in the effect, which scripts
    /// find it by.
    pub effect: usize,
    pub pass: usize,
    /// Whether its effect is shown at first.
    pub visible: bool,
}

/// The most passes a layer's effects may have.
const MOST_PASSES: usize = 32;

/// Effects hidden at first that can't be drawn, by place in the layer's list, and why.
pub type Undrawn = Vec<(usize, String)>;

const INSTANCE: &[&str] = &["file", "id", "name", "passes", "username", "visible"];
const DEFINITION: &[&str] = &[
    "dependencies",
    "description",
    "editable",
    "gizmos",
    "group",
    "name",
    "passes",
    "preview",
    "replacementkey",
    "version",
];
const STEP: &[&str] = &["material"];
const OVERRIDE: &[&str] = &["combos", "constantshadervalues", "id", "textures"];
const PASS: &[&str] = &[
    "alphawriting",
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
    /// The passes of an image's effects, in drawing order. Shown effects are prepared first,
    /// so hidden ones only get what they leave of the budget and the passes a layer may have;
    /// a hidden effect that can't be drawn is left out and named in the second list with why,
    /// for scripts that would show it to fail.
    pub(super) fn effects(
        &self,
        image: &Image,
        resources: &mut Resources,
        remaining: &mut usize,
        still: bool,
    ) -> Result<(Vec<EffectPass>, Undrawn), Error> {
        let mut made: Vec<Vec<EffectPass>> = Vec::new();
        let mut undrawn = Vec::new();
        let mut count = 0;
        let (shown, hidden): (Vec<_>, Vec<_>) =
            image.effects.iter().partition(|effect| effect.visible);
        for effect in shown {
            let passes = self.passes(effect, resources, remaining, still)?;
            count += passes.len();
            if count > MOST_PASSES {
                return Err(Error::Format(
                    "a layer with more than 32 effect passes".into(),
                ));
            }
            made.push(passes);
        }
        for effect in hidden {
            match self.passes(effect, resources, remaining, still) {
                Ok(passes) if count + passes.len() <= MOST_PASSES => {
                    count += passes.len();
                    made.push(passes);
                }
                Ok(_) => undrawn.push((
                    effect.index,
                    "a layer with more than 32 effect passes".into(),
                )),
                Err(error) => undrawn.push((effect.index, error.to_string())),
            }
        }
        let mut passes: Vec<EffectPass> = made.into_iter().flatten().collect();
        passes.sort_by_key(|pass| (pass.effect, pass.pass));
        Ok((passes, undrawn))
    }

    /// One effect's passes, each knowing which it is.
    fn passes(
        &self,
        effect: &crate::scene::Effect,
        resources: &mut Resources,
        remaining: &mut usize,
        still: bool,
    ) -> Result<Vec<EffectPass>, Error> {
        let file = text(effect.value.get("file")).unwrap_or_default();
        let mut passes = Vec::new();
        self.effect(
            &effect.value,
            file,
            resources,
            remaining,
            still,
            &mut passes,
        )
        .map_err(|error| Error::Format(format!("effect {}: {error}", name(file))))?;
        Ok(passes
            .into_iter()
            .enumerate()
            .map(|(at, pass)| EffectPass {
                effect: effect.index,
                pass: at,
                visible: effect.visible,
                ..pass
            })
            .collect())
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
        // Timelines play on the scene's own values, not on an effect material's.
        if constants.values().any(|value| changing(value, &[])) {
            return Err(Error::Format(
                "an effect material's own animated values need another renderer".into(),
            ));
        }
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
        if combos.values().any(|value| changing(value, &[]))
            || constants
                .values()
                .any(|value| changing(value, &["script", "scriptproperties", "animation"]))
        {
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
        Ok(EffectPass {
            fragment,
            inputs,
            effect: 0,
            pass: 0,
            visible: true,
        })
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

/// A value with more in it than a property binding and the `run` keys, or a timeline that
/// can't be played: an animation or a script where none runs.
fn changing(value: &Value, run: &[&str]) -> bool {
    value.as_object().is_some_and(|fields| {
        fields
            .keys()
            .any(|key| !["value", "user"].contains(&key.as_str()) && !run.contains(&key.as_str()))
            || fields
                .get("animation")
                .is_some_and(|animation| crate::scene::Timeline::parse(animation).is_err())
    })
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
