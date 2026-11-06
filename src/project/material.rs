use super::*;
use crate::shader::{Fragment, Template};
use serde_json::Map;

#[derive(Default)]
pub(super) struct Resources {
    /// Textures by file, and by whether they were read as stills, which refuses animations.
    pub textures: HashMap<(String, bool), Result<Arc<Texture>, String>>,
    materials: HashMap<String, Result<Material, String>>,
    source_bytes: usize,
    custom_materials: usize,
}

#[derive(Clone)]
pub(super) struct Material {
    pub names: Vec<Option<String>>,
    pub fragment: Option<Arc<Fragment>>,
    pub blending: Blending,
    /// What a plain image material multiplies the layer's brightness and alpha by.
    pub shade: [f32; 2],
    /// A plain colour with no texture, which the layer's colour and size make.
    pub solid: bool,
    /// A plain image material that plays its texture's frames in turn.
    pub sprite: bool,
    /// A composition layer: its picture is the scene drawn so far, under it.
    pub compose: bool,
}

impl Project {
    pub(super) fn texture(
        &self,
        name: &str,
        resources: &mut Resources,
        remaining: &mut usize,
        still: bool,
    ) -> Result<Arc<Texture>, Error> {
        let file = texture_file(name)?;
        if let Some(cached) = resources.textures.get(&(file.clone(), still)) {
            return cached
                .as_ref()
                .map(Arc::clone)
                .map_err(|message| Error::Format(message.clone()));
        }
        let result = (|| {
            let bytes = self
                .try_file(&file, crate::files::RESOURCE_BYTES)?
                .ok_or_else(|| {
                    Error::Format(format!("{file} isn't in the project or the assets"))
                })?;
            let texture = if still {
                Texture::parse_still_with_budget(&bytes, *remaining)
            } else {
                Texture::parse_with_budget(&bytes, *remaining)
            }
            .map_err(|error| Error::Format(format!("{file}: {error}")))?;
            let bytes: usize = texture
                .images
                .iter()
                .flatten()
                .map(|mip| mip.pixels.len())
                .sum();
            *remaining -= bytes;
            Ok(Arc::new(texture))
        })()
        .map_err(|error: Error| diagnostic(error));
        let returned = result
            .as_ref()
            .map(Arc::clone)
            .map_err(|message| Error::Format(message.clone()));
        resources.textures.insert((file, still), result);
        returned
    }

    /// Material `name` as a layer's `instance` changes it.
    pub(super) fn instanced(
        &self,
        name: &str,
        instance: Option<&Map<String, Value>>,
        resources: &mut Resources,
    ) -> Result<Material, Error> {
        if !relative(name) {
            return Err(Error::Format(
                "material path must stay within its resource root".into(),
            ));
        }
        let key = match instance {
            Some(instance) => format!("{name}\u{0}{}", Value::Object(instance.clone())),
            None => name.to_owned(),
        };
        if let Some(cached) = resources.materials.get(&key) {
            return cached
                .as_ref()
                .cloned()
                .map_err(|message| Error::Format(message.clone()));
        }
        if resources.materials.len() >= 4096 {
            return Err(Error::Format(
                "scene exceeds 4096 material resources".into(),
            ));
        }
        let result = self
            .read_material(name, instance, resources)
            .map_err(diagnostic);
        let returned = result
            .as_ref()
            .cloned()
            .map_err(|message| Error::Format(message.clone()));
        resources.materials.insert(key, result);
        returned
    }

    fn read_material(
        &self,
        name: &str,
        instance: Option<&Map<String, Value>>,
        resources: &mut Resources,
    ) -> Result<Material, Error> {
        let mut material = self.json(name)?;
        if let Some(instance) = instance {
            lay_over(&mut material, instance)?;
        }
        if scripted(&material) {
            return Err(Error::Format(
                "a scripted material needs another renderer".into(),
            ));
        }
        let passes = material
            .get("passes")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Format("a material without passes".into()))?;
        if passes.len() != 1 {
            return Err(Error::Format("a material needs more than one pass".into()));
        }
        let pass = &passes[0];
        let shader = match pass.get("shader") {
            None => "genericimage",
            Some(value) => text(Some(value))
                .ok_or_else(|| Error::Format("material shader must be a string".into()))?,
        };
        if let Some(field) = pass.as_object().and_then(|pass| {
            pass.keys().find(|key| {
                // Culling and depth say nothing to a flat layer drawn in order.
                !matches!(
                    key.as_str(),
                    "shader"
                        | "alphawriting"
                        | "textures"
                        | "combos"
                        | "constantshadervalues"
                        | "blending"
                        | "cullmode"
                        | "depthtest"
                        | "depthwrite"
                        | "usertextures"
                )
            })
        }) {
            return Err(Error::Format(format!(
                "unsupported material pass field {}",
                field.chars().take(160).collect::<String>()
            )));
        }
        let blending = match pass.get("blending") {
            None => Blending::Translucent,
            Some(value) => match text(Some(value))
                .ok_or_else(|| Error::Format("material blending must be a string".into()))?
            {
                "normal" | "disabled" => Blending::Opaque,
                "translucent" => Blending::Translucent,
                "additive" => Blending::Additive,
                _ => return Err(Error::Format("unsupported material blending mode".into())),
            },
        };
        self.user_textures(pass.get("usertextures"))?;
        let combos = object(pass.get("combos"), 64, 64)?;
        let constants = object(pass.get("constantshadervalues"), 128, 128)?;
        let mut names = names(pass.get("textures"))?;
        // Wallpaper Engine's plain image shaders, in their versions, draw what the built-in
        // layer shader draws when nothing varies them.
        if shader == "flat" && names.iter().all(Option::is_none) {
            if !combos.is_empty() || !constants.is_empty() {
                return Err(Error::Format(
                    "flat colour options need another renderer".into(),
                ));
            }
            return Ok(Material {
                names: Vec::new(),
                fragment: None,
                blending,
                shade: [1.0; 2],
                solid: true,
                sprite: false,
                compose: false,
            });
        }
        // Wallpaper Engine's composition layers: what's been drawn under them, which their
        // effects change.
        if matches!(shader, "composelayer" | "passthrough")
            && names == [Some("_rt_FullFrameBuffer".to_owned())]
            && constants.is_empty()
            && combos
                .values()
                .all(|value| crate::value::number(Some(value)) == Some(0.0))
        {
            return Ok(Material {
                names: Vec::new(),
                fragment: None,
                blending,
                shade: [1.0; 2],
                solid: false,
                sprite: false,
                compose: true,
            });
        }
        if matches!(
            shader,
            "genericimage" | "genericimage2" | "genericimage3" | "genericimage4"
        ) {
            let (shade, sprite) = plain(&combos, &constants)?;
            if names.len() != 1 {
                return Err(Error::Format(
                    "genericimage needs exactly one texture slot".into(),
                ));
            }
            bindings(&names)?;
            return Ok(Material {
                names,
                fragment: None,
                blending,
                shade,
                solid: false,
                sprite,
                compose: false,
            });
        }
        let fragment = self.custom(shader, &combos, &constants, &mut names, None, resources)?;
        Ok(Material {
            names,
            fragment: Some(fragment),
            blending,
            shade: [1.0; 2],
            solid: false,
            sprite: false,
            compose: false,
        })
    }

    /// Textures the user or the system may give in place of a material's own: album art, which
    /// a desktop without playing media doesn't have, and pictures the project's properties
    /// let the user choose, of which none chosen leaves the material's.
    pub(super) fn user_textures(&self, given: Option<&Value>) -> Result<(), Error> {
        let Some(given) = given else {
            return Ok(());
        };
        let slots = given
            .as_array()
            .filter(|slots| slots.len() <= 8)
            .ok_or_else(|| Error::Format("user textures must be at most eight slots".into()))?;
        for slot in slots {
            let chosen = match slot {
                Value::Null => continue,
                Value::Object(system) if text(system.get("type")) == Some("system") => continue,
                Value::String(property) => self
                    .values
                    .get(property)
                    .is_some_and(|value| value.as_str().is_none_or(|path| !path.is_empty())),
                _ => true,
            };
            if chosen {
                return Err(Error::Format(
                    "textures the user chose need another renderer".into(),
                ));
            }
        }
        Ok(())
    }

    /// A custom shader's program for one pass. An effect's pass has slots the chain `provides`
    /// rather than textures: the layer's picture so far, or one of its effect's buffers.
    pub(super) fn custom(
        &self,
        shader: &str,
        combos: &serde_json::Map<String, Value>,
        constants: &serde_json::Map<String, Value>,
        names: &mut Vec<Option<String>>,
        provides: Option<u8>,
        resources: &mut Resources,
    ) -> Result<Arc<Fragment>, Error> {
        if resources.custom_materials >= 256 {
            return Err(Error::Format(
                "scene exceeds 256 custom shader passes".into(),
            ));
        }
        resources.custom_materials += 1;
        let shader = shader.strip_prefix("shaders/").unwrap_or(shader);
        if shader.len() > 4078 || !relative(shader) {
            return Err(Error::Format(
                "shader resource name must stay within shaders and its path limit".into(),
            ));
        }
        let entry = if shader.ends_with(".frag") {
            shader.to_owned()
        } else {
            format!("{shader}.frag")
        };
        let vertex_entry = format!("{}.vert", entry.strip_suffix(".frag").unwrap_or(&entry));
        let load = |name: &str| {
            self.try_file(name, 256 * 1024)?
                .map(|bytes| {
                    std::str::from_utf8(&bytes)
                        .map(str::to_owned)
                        .map_err(|_| Error::Format(format!("{name}: shader source is not UTF-8")))
                })
                .transpose()
        };
        // Every stock shader comes with a vertex shader; it's prepared with the fragment and
        // the material is refused when it can't be, rather than stood in for.
        let vertex = match self.try_file(&format!("shaders/{vertex_entry}"), 256 * 1024)? {
            Some(_) => Some(Template::load(&vertex_entry, load)?),
            None => None,
        };
        let template = Template::load(&entry, load)?;
        if let Some(provides) = provides {
            let highest = 8 - provides.leading_zeros() as usize;
            if names.len() < highest {
                names.resize(highest, None);
            }
        }
        template.fill_defaults(names, combos)?;
        let mask = if let Some(provides) = provides {
            let mut mask = provides;
            for (slot, name) in names.iter_mut().enumerate() {
                if provides & (1 << slot) != 0 {
                    *name = None;
                } else if let Some(name) = name {
                    texture_file(name)?;
                    mask |= 1 << slot;
                }
            }
            mask
        } else {
            bindings(names)?
        };
        let fragment = template.prepare(combos, constants, Some(mask), vertex)?;
        fragment.validate_inputs(mask)?;
        if fragment.unplaced && provides.is_none() {
            return Err(Error::Format(
                "a vertex shader that places a_Position without the matrix".into(),
            ));
        }
        let bytes = fragment.source.len() + fragment.vertex.as_ref().map_or(0, String::len);
        if bytes > (16 * 1024 * 1024usize).saturating_sub(resources.source_bytes) {
            return Err(Error::Format("scene shader sources exceed 16 MiB".into()));
        }
        resources.source_bytes += bytes;
        Ok(Arc::new(fragment))
    }
}

/// Wallpaper Engine's plain image shaders, in all their versions, draw what the built-in layer
/// shader draws while lighting, reflection and fog are off, and play a sprite sheet's frames.
/// Their `Brightness` and `Alpha` values come back as multipliers for the layer's own, with
/// whether the sheet plays. Older materials name combos in lower case.
fn plain(
    combos: &serde_json::Map<String, Value>,
    constants: &serde_json::Map<String, Value>,
) -> Result<([f32; 2], bool), Error> {
    let mut sprite = false;
    for (name, value) in combos {
        let value = crate::value::number(Some(value));
        let quiet = match name.to_ascii_uppercase().as_str() {
            "VERSION" => true,
            "LIGHTING" | "REFLECTION" | "FOG" => value == Some(0.0),
            "SPRITESHEET" if matches!(value, Some(0.0) | Some(1.0)) => {
                sprite = value == Some(1.0);
                true
            }
            _ => false,
        };
        if !quiet {
            let name: String = name.chars().take(32).collect();
            return Err(Error::Format(format!(
                "genericimage with {name} needs another renderer"
            )));
        }
    }
    let mut shade = [1.0; 2];
    for (name, value) in constants {
        let slot = match name.as_str() {
            "Brightness" => 0,
            "Alpha" => 1,
            _ => {
                return Err(Error::Format(
                    "genericimage values other than brightness and alpha".into(),
                ));
            }
        };
        shade[slot] = crate::value::number(Some(value))
            .filter(|value| value.is_finite())
            .ok_or_else(|| Error::Format("genericimage values must be numbers".into()))?;
    }
    Ok((shade, sprite))
}

/// A layer's `instance` laid over its material's one pass: its combos and values added to the
/// material's, each texture it names in place of the material's in that slot, and its user
/// textures in place of the material's.
fn lay_over(material: &mut Value, instance: &Map<String, Value>) -> Result<(), Error> {
    if let Some(key) = instance.keys().find(|key| {
        !matches!(
            key.as_str(),
            "combos" | "constantshadervalues" | "id" | "textures" | "usertextures"
        )
    }) {
        let key: String = key.chars().take(64).collect();
        return Err(Error::Format(format!(
            "unsupported material instance field {key}"
        )));
    }
    let Some(Value::Object(pass)) = material
        .get_mut("passes")
        .and_then(Value::as_array_mut)
        .filter(|passes| passes.len() == 1)
        .and_then(|passes| passes.first_mut())
    else {
        return Err(Error::Format(
            "an instance of a material without one pass".into(),
        ));
    };
    for key in ["combos", "constantshadervalues"] {
        let Some(given) = instance.get(key) else {
            continue;
        };
        let given = given
            .as_object()
            .ok_or_else(|| Error::Format(format!("instance {key} must be an object")))?;
        let own = pass.entry(key).or_insert_with(|| Value::Object(Map::new()));
        let own = own
            .as_object_mut()
            .ok_or_else(|| Error::Format(format!("material {key} must be an object")))?;
        own.extend(given.clone());
    }
    if let Some(given) = instance.get("textures") {
        let given = given
            .as_array()
            .filter(|slots| slots.len() <= 8)
            .ok_or_else(|| Error::Format("instance textures must be at most eight slots".into()))?;
        let own = pass
            .entry("textures")
            .or_insert_with(|| Value::Array(Vec::new()));
        let own = own
            .as_array_mut()
            .ok_or_else(|| Error::Format("material textures must be a list".into()))?;
        for (slot, name) in given.iter().enumerate() {
            if name.is_null() {
                continue;
            }
            if own.len() <= slot {
                own.resize(slot + 1, Value::Null);
            }
            own[slot] = name.clone();
        }
    }
    if let Some(given) = instance.get("usertextures") {
        pass.insert("usertextures".into(), given.clone());
    }
    Ok(())
}

pub(super) fn object(
    value: Option<&Value>,
    count: usize,
    key_bytes: usize,
) -> Result<serde_json::Map<String, Value>, Error> {
    match value {
        None => Ok(serde_json::Map::new()),
        Some(Value::Object(object))
            if object.len() <= count && object.keys().all(|key| key.len() <= key_bytes) =>
        {
            Ok(object.clone())
        }
        Some(Value::Object(_)) => Err(Error::Format(
            "shader parameter object exceeds its key/count limits".into(),
        )),
        Some(_) => Err(Error::Format(
            "shader combos and constants must be objects".into(),
        )),
    }
}

pub(super) fn names(value: Option<&Value>) -> Result<Vec<Option<String>>, Error> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .filter(|values| values.len() <= 8)
        .ok_or_else(|| {
            Error::Format("material textures must be an array of at most eight slots".into())
        })?;
    values
        .iter()
        .map(|value| match value {
            Value::Null => Ok(None),
            Value::String(name) if name.is_empty() => Ok(None),
            Value::String(name) => {
                // Another layer's finished picture, or the scene's under a composition layer,
                // is a name the renderer resolves, not a file.
                if crate::scene::composite(name).is_none() && name != "_rt_FullFrameBuffer" {
                    texture_file(name)?;
                }
                Ok(Some(name.clone()))
            }
            _ => Err(Error::Format(
                "material texture must be a resource name or null".into(),
            )),
        })
        .collect()
}

fn bindings(names: &[Option<String>]) -> Result<u8, Error> {
    if names.first().is_none_or(Option::is_none) {
        return Err(Error::Format(
            "image material must bind texture slot zero".into(),
        ));
    }
    let mut mask = 0;
    for (slot, name) in names.iter().enumerate() {
        if let Some(name) = name {
            texture_file(name)?;
            mask |= 1 << slot;
        }
    }
    Ok(mask)
}

fn texture_file(name: &str) -> Result<String, Error> {
    if name.starts_with("_rt_") {
        return Err(Error::Format(
            "render-target texture inputs require another renderer".into(),
        ));
    }
    if name.len() > 4082 || !relative(name) {
        return Err(Error::Format(
            "texture resource name must stay within materials and its path limit".into(),
        ));
    }
    Ok(format!("materials/{name}.tex"))
}

#[cfg(test)]
mod instance_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_instance_adds_values_and_swaps_the_textures_it_names() {
        let mut material = json!({"passes": [{"shader": "genericimage2", "combos": {"VERSION": 2},
            "textures": ["util/white", "mask"]}]});
        let instance = json!({"id": 4, "combos": {"TINT": 1}, "textures": [null, "other"],
            "usertextures": [{"type": "system", "name": "$mediaThumbnail"}]});
        lay_over(&mut material, instance.as_object().unwrap()).unwrap();
        let pass = &material["passes"][0];
        assert_eq!(pass["combos"], json!({"VERSION": 2, "TINT": 1}));
        assert_eq!(pass["textures"], json!(["util/white", "other"]));
        assert_eq!(pass["usertextures"][0]["type"], "system");
        let odd = json!({"shader": "x"});
        assert!(lay_over(&mut material, odd.as_object().unwrap()).is_err());
    }
}
