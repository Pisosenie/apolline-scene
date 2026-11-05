//! A Wallpaper Engine project folder. `project.json` names the scene; the scene's files are in
//! `scene.pkg` or loose in the folder, and what neither has comes from Wallpaper Engine's own
//! `assets` folder, which holds the materials and shaders scenes share.

mod effect;
mod material;
mod particle;

pub use effect::EffectPass;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Component, Path};
use std::sync::Arc;

use serde_json::Value;

use crate::Error;
use crate::pkg::Package;
use crate::properties::{self, Schema, Values};
use crate::scene::{Image, Missing, Object, Scene};
use crate::tex::Texture;
use crate::value::{flag, text};

pub struct Project {
    folder: crate::files::Root,
    package: Option<Package>,
    assets: Option<crate::files::Root>,
    pub title: String,
    scene: String,
    properties: Schema,
    values: Values,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blending {
    /// Covers what is under it.
    Opaque,
    /// Blends by its alpha.
    Translucent,
    /// Adds its light to what is under it.
    Additive,
}

/// An image layer with what it takes to draw it.
pub struct Layer {
    pub image: Image,
    pub texture: Arc<Texture>,
    pub secondary: Vec<TextureInput>,
    pub fragment: Option<Arc<crate::shader::Fragment>>,
    pub blending: Blending,
    /// It covers the whole scene, whatever its own origin and size say.
    pub fullscreen: bool,
    /// Its effects' passes, drawn in order over its picture before it's placed in the scene.
    pub effects: Vec<EffectPass>,
    /// A particle system drawn with its texture, in place of a picture.
    pub particles: Option<Arc<crate::particles::System>>,
}

/// A texture bound to an explicit secondary sampler slot.
#[derive(Clone)]
pub struct TextureInput {
    pub slot: u8,
    pub texture: Arc<Texture>,
}

/// A complete supported 2D scene. Unsupported features are refused before a host replaces its
/// current wallpaper, rather than silently leaving parts of the scene out.
pub struct Prepared {
    pub scene: Scene,
    pub layers: Vec<Layer>,
}

#[derive(Clone)]
struct Resolved {
    texture: Arc<Texture>,
    secondary: Vec<TextureInput>,
    fragment: Option<Arc<crate::shader::Fragment>>,
    blending: Blending,
    fullscreen: bool,
    shade: [f32; 2],
}

impl Project {
    /// `assets` is Wallpaper Engine's `assets` folder, when it's on this machine.
    pub fn open(folder: &Path, assets: Option<&Path>) -> Result<Self, Error> {
        let folder = crate::files::Root::open(folder)?;
        let project: Value = serde_json::from_slice(
            &folder
                .read("project.json", crate::files::JSON_BYTES)?
                .ok_or_else(|| Error::Format("project.json isn't in the project".into()))?,
        )
        .map_err(|err| Error::Format(format!("project.json: {err}")))?;
        let kind = text(project.get("type")).unwrap_or("scene");
        if !kind.eq_ignore_ascii_case("scene") {
            return Err(Error::Format(format!("a {kind} wallpaper, not a scene")));
        }
        let package = folder
            .read("scene.pkg", crate::files::PACKAGE_BYTES)?
            .map(Package::parse)
            .transpose()?;
        let properties = Schema::parse(&project)?;
        let values = properties.resolve(&Values::new())?;
        Ok(Project {
            properties,
            values,
            folder,
            package,
            assets: assets.map(crate::files::Root::open).transpose()?,
            title: text(project.get("title"))
                .unwrap_or_default()
                .trim()
                .to_owned(),
            scene: text(project.get("file")).unwrap_or("scene.json").to_owned(),
        })
    }

    pub fn properties(&self) -> &Schema {
        &self.properties
    }

    /// Validates a complete set of overrides before replacing the current values.
    pub fn set_properties(&mut self, overrides: &Values) -> Result<(), Error> {
        self.values = self.properties.resolve(overrides)?;
        Ok(())
    }

    /// A file by its path inside the project, from the package, the folder or the assets.
    pub fn file(&self, name: &str) -> Option<Cow<'_, [u8]>> {
        self.try_file(name, crate::files::RESOURCE_BYTES)
            .ok()
            .flatten()
    }

    fn try_file(&self, name: &str, limit: usize) -> Result<Option<Cow<'_, [u8]>>, Error> {
        if name.len() > 4096 {
            return Err(Error::Format("resource path exceeds 4096 bytes".into()));
        }
        if !relative(name) {
            return Err(Error::Format(format!(
                "{name}: path must stay inside the project or assets"
            )));
        }
        if let Some(bytes) = self.package.as_ref().and_then(|package| package.get(name)) {
            if bytes.len() > limit {
                return Err(Error::Format(format!(
                    "{name}: packaged resource exceeds {limit} bytes"
                )));
            }
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        // A file the project can't give, too big or behind a link, may still be in the assets.
        let mut refused = None;
        for root in [Some(&self.folder), self.assets.as_ref()]
            .into_iter()
            .flatten()
        {
            match root.read(name, limit) {
                Ok(Some(bytes)) => return Ok(Some(Cow::Owned(bytes))),
                Ok(None) => {}
                Err(err) => {
                    refused.get_or_insert(err);
                }
            }
        }
        refused.map_or(Ok(None), Err)
    }

    fn json(&self, name: &str) -> Result<Value, Error> {
        let bytes = self
            .try_file(name, crate::files::JSON_BYTES)?
            .ok_or_else(|| Error::Format(format!("{name} isn't in the project")))?;
        let mut json = serde_json::from_slice(&bytes)
            .map_err(|err| Error::Format(format!("{name}: {err}")))?;
        properties::bind(&mut json, &self.values);
        Ok(json)
    }

    pub fn scene(&self) -> Result<Scene, Error> {
        Scene::from_value(&self.json(&self.scene)?)
    }

    pub fn prepare_still(&self) -> Result<Prepared, Error> {
        let prepared = self.prepare()?;
        if prepared.layers.iter().any(|layer| {
            layer
                .fragment
                .iter()
                .chain(layer.effects.iter().map(|pass| &pass.fragment))
                .any(|fragment| fragment.animated || fragment.reads.audio)
                || layer.particles.is_some()
        }) {
            return Err(Error::Format(
                "time-dependent materials need animated scene playback".into(),
            ));
        }
        Ok(prepared)
    }

    /// Supported 2D scenes with static textures and optional time-driven fragment materials.
    pub fn prepare(&self) -> Result<Prepared, Error> {
        self.prepare_inner(false)
    }

    /// A complete deterministic frame; pointer parallax and shaders that read the pointer see
    /// it at the centre until the host moves it with `Renderer::follow`.
    pub fn prepare_snapshot(&self) -> Result<Prepared, Error> {
        self.prepare_inner(true)
    }

    fn prepare_inner(&self, allow_parallax: bool) -> Result<Prepared, Error> {
        let root = self.json(&self.scene)?;
        if scripted(&root) {
            return Err(Error::Format("SceneScript needs another renderer".into()));
        }
        let scene = Scene::from_value(&root)?;
        let mut missing = scene.missing();
        if !allow_parallax && scene.parallax.is_some() {
            missing.push(Missing::PointerParallax);
        }
        if !missing.is_empty() {
            let named: Vec<String> = missing.iter().map(ToString::to_string).collect();
            return Err(Error::Format(format!("needs {}", named.join(", "))));
        }
        let (layers, skipped) = self.prepare_layers(&scene, 512 * 1024 * 1024, true);
        if !skipped.is_empty() {
            return Err(Error::Format(skipped.join("; ")));
        }
        let pointed = layers.iter().any(|layer| {
            layer
                .fragment
                .iter()
                .chain(layer.effects.iter().map(|pass| &pass.fragment))
                .any(|fragment| fragment.reads.pointer || fragment.reads.parallax)
        });
        if !allow_parallax && pointed {
            return Err(Error::Format(format!("needs {}", Missing::PointerParallax)));
        }
        Ok(Prepared { scene, layers })
    }

    /// The scene's image layers, each with its texture. A layer whose files are missing or
    /// broken is left out and named in the second list, so the rest still draws.
    pub fn layers(&self, scene: &Scene) -> (Vec<Layer>, Vec<String>) {
        self.layers_with_budget(scene, 512 * 1024 * 1024)
    }

    fn layers_with_budget(&self, scene: &Scene, remaining: usize) -> (Vec<Layer>, Vec<String>) {
        self.prepare_layers(scene, remaining, false)
    }

    fn prepare_layers(
        &self,
        scene: &Scene,
        mut remaining: usize,
        still: bool,
    ) -> (Vec<Layer>, Vec<String>) {
        let mut layers = Vec::new();
        let mut skipped = Vec::new();
        let mut omitted = 0;
        let mut textures = material::Resources::default();
        let mut models = HashMap::new();
        let shown: Vec<&Object> = scene
            .objects
            .iter()
            .filter(|object| match object {
                Object::Image(image) => image.visible,
                Object::Particles(particles) => particles.visible,
                _ => false,
            })
            .collect();
        if shown.len() > 4096 {
            skipped.push("scene exceeds 4096 layers".into());
        }
        for object in shown.into_iter().take(4096) {
            let (name, layer) = match object {
                Object::Image(image) => (
                    &image.name,
                    self.layer(image, &mut models, &mut textures, &mut remaining, still),
                ),
                Object::Particles(particles) => (
                    &particles.name,
                    self.particle_layer(particles, &mut textures, &mut remaining),
                ),
                _ => continue,
            };
            match layer {
                Ok(layer) => layers.push(layer),
                Err(err) => {
                    if skipped.len() < 64 {
                        let name: String = name.chars().take(160).collect();
                        let reason: String = diagnostic(&err);
                        skipped.push(format!("{name}: {reason}"));
                    } else {
                        omitted += 1;
                    }
                }
            }
        }
        if omitted > 0 {
            skipped.push(format!("{omitted} additional layer errors omitted"));
        }
        (layers, skipped)
    }

    fn layer(
        &self,
        image: &Image,
        models: &mut HashMap<String, Result<Resolved, String>>,
        textures: &mut material::Resources,
        remaining: &mut usize,
        still: bool,
    ) -> Result<Layer, Error> {
        let resolved = models
            .entry(image.model.clone())
            .or_insert_with(|| {
                self.resolve(&image.model, textures, remaining, still)
                    .map_err(diagnostic)
            })
            .as_ref()
            .map_err(|reason| Error::Format(reason.clone()))?
            .clone();
        let effects = self.effects(image, textures, remaining, still)?;
        let mut image = image.clone();
        image.brightness *= resolved.shade[0];
        image.alpha *= resolved.shade[1];
        Ok(Layer {
            image,
            texture: Arc::clone(&resolved.texture),
            secondary: resolved.secondary.clone(),
            fragment: resolved.fragment.clone(),
            blending: resolved.blending,
            fullscreen: resolved.fullscreen,
            effects,
            particles: None,
        })
    }

    /// Resolve each model once per preparation, including failures of shared resources.
    fn resolve(
        &self,
        model_name: &str,
        textures: &mut material::Resources,
        remaining: &mut usize,
        still: bool,
    ) -> Result<Resolved, Error> {
        let model = self.json(model_name)?;
        if scripted(&model) {
            return Err(Error::Format(
                "a scripted model needs another renderer".into(),
            ));
        }
        let material = text(model.get("material"))
            .ok_or_else(|| Error::Format(format!("{model_name} names no material")))?;
        let material = self.material(material, textures)?;
        let names = material.names;
        let fragment = material.fragment;
        let shade = material.shade;
        let mut decoded = Vec::with_capacity(names.len());
        for (slot, name) in names.into_iter().enumerate() {
            if let Some(name) = name {
                decoded.push(TextureInput {
                    slot: slot as u8,
                    texture: self.texture(&name, textures, remaining, still)?,
                });
            }
        }
        let texture = decoded.remove(0).texture;
        Ok(Resolved {
            texture,
            secondary: decoded,
            fragment,
            blending: material.blending,
            fullscreen: flag(model.get("fullscreen")).unwrap_or(false),
            shade,
        })
    }
}

fn scripted(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields.keys().any(|key| key.eq_ignore_ascii_case("script"))
                || fields.values().any(scripted)
        }
        Value::Array(items) => items.iter().any(scripted),
        _ => false,
    }
}

fn diagnostic(error: impl std::fmt::Display) -> String {
    error.to_string().chars().take(1800).collect()
}

fn relative(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 4096
        && !name.contains(['\\', ':', '\0'])
        && Path::new(name)
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

#[cfg(test)]
mod tests;
