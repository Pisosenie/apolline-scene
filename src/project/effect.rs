//! Effects on a layer. Each instance names an `effect.json`, whose passes name materials, and
//! its own passes override their combos, values and textures. A pass draws over the layer's
//! picture so far, which it reads in texture slot 0, or into one of the effect's buffers
//! (`fbos`, a half or a quarter the picture's size, say), reading the picture and the buffers
//! in the slots its `bind` says. Scripts may show and hide effects and change their values as
//! the scene runs. What isn't understood — copies, passes that blend, buffers of other
//! formats — is refused by name.

use super::material::{Resources, names, object};
use super::*;
use crate::shader::Fragment;
use crate::value::number;

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
    /// The effect's buffer it draws into; none for the layer's picture.
    pub target: Option<Buffer>,
    /// What it reads besides its textures, by slot.
    pub reads: Vec<(u8, Source)>,
    /// Other layers' finished pictures it reads, by slot and the layer's object id.
    pub composites: Vec<(u8, i64)>,
}

/// One of an effect's buffers: its place among them, and how many times smaller than the
/// layer's picture it is each way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buffer {
    pub at: usize,
    pub scale: u32,
}

/// What an effect pass reads: the layer's picture before the pass, or one of its effect's
/// buffers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Previous,
    Buffer(Buffer),
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
    "fbos",
    "gizmos",
    "group",
    "name",
    "passes",
    "performance",
    "preview",
    "replacementkey",
    "version",
];
const STEP: &[&str] = &["bind", "material", "target"];
const FBO: &[&str] = &["format", "name", "scale"];
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
        let buffers = buffers(&definition)?;
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
            let buffer = |name: &str| {
                buffers
                    .iter()
                    .position(|(named, _)| named == name)
                    .map(|at| Buffer {
                        at,
                        scale: buffers[at].1,
                    })
                    .ok_or_else(|| {
                        Error::Format(format!(
                            "a pass reads or draws {}, which isn't one of its buffers",
                            name.chars().take(64).collect::<String>()
                        ))
                    })
            };
            let target = text(step.get("target")).map(buffer).transpose()?;
            let mut reads = Vec::new();
            for bound in step
                .get("bind")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let slot = number(bound.get("index"))
                    .filter(|slot| (0.0..8.0).contains(slot) && slot.fract() == 0.0)
                    .ok_or_else(|| Error::Format("a pass binds outside slots 0 to 7".into()))?
                    as u8;
                let source = match text(bound.get("name")) {
                    Some("previous") => Source::Previous,
                    Some(name) => Source::Buffer(buffer(name)?),
                    None => return Err(Error::Format("a pass binds nothing by name".into())),
                };
                if reads.iter().any(|(taken, _)| *taken == slot) {
                    return Err(Error::Format("a pass binds one slot twice".into()));
                }
                reads.push((slot, source));
            }
            // Slot 0 is the picture so far unless a bind says otherwise.
            if !reads.iter().any(|(slot, _)| *slot == 0) {
                reads.push((0, Source::Previous));
            }
            let provides = reads.iter().fold(0u8, |mask, (slot, _)| mask | 1 << slot);
            let pass = self.effect_pass(material, over, provides, resources, remaining, still)?;
            // Only a pass into a buffer is drawn as a quad already in place.
            if pass.fragment.unplaced && target.is_none() {
                return Err(Error::Format(
                    "a vertex shader that places a_Position without the matrix".into(),
                ));
            }
            passes.push(EffectPass {
                target,
                reads,
                ..pass
            });
        }
        Ok(())
    }

    fn effect_pass(
        &self,
        material: &str,
        over: Option<&Value>,
        provides: u8,
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
                    if provides & (1 << slot) != 0 {
                        // The editor names the layer's own picture where the pass binds the
                        // picture before it, which is the same; the bind stands.
                        if crate::scene::composite(&name).is_some() {
                            continue;
                        }
                        return Err(Error::Format(
                            "an effect binds a texture where its pass reads a buffer".into(),
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
        // Another layer's finished picture is drawn, not read from a file.
        let mut composites = Vec::new();
        for (slot, name) in textures.iter_mut().enumerate() {
            if let Some(id) = name.as_deref().and_then(crate::scene::composite) {
                composites.push((slot as u8, id));
                *name = None;
            }
        }
        let provides = composites
            .iter()
            .fold(provides, |mask, (slot, _)| mask | 1 << slot);
        let fragment = self.custom(
            shader,
            &combos,
            &constants,
            &mut textures,
            Some(provides),
            resources,
        )?;
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
            target: None,
            reads: Vec::new(),
            composites,
        })
    }
}

/// An effect's buffers, by name, and how many times smaller than the picture each is.
fn buffers(definition: &Value) -> Result<Vec<(String, u32)>, Error> {
    let mut found = Vec::new();
    for fbo in definition
        .get("fbos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        known(fbo, FBO)?;
        if !matches!(
            text(fbo.get("format")),
            None | Some("rgba_backbuffer" | "rgba8888")
        ) {
            return Err(Error::Format("effect buffers of other formats".into()));
        }
        let scale = number(fbo.get("scale"))
            .unwrap_or(1.0)
            .clamp(1.0, 16.0)
            .round() as u32;
        let name = text(fbo.get("name"))
            .ok_or_else(|| Error::Format("an effect buffer without a name".into()))?;
        found.push((name.to_owned(), scale));
        if found.len() > 8 {
            return Err(Error::Format("an effect with more than 8 buffers".into()));
        }
    }
    Ok(found)
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
