use super::*;
use crate::shader::{Fragment, Template};

#[derive(Default)]
pub(super) struct Resources {
    pub textures: HashMap<String, Result<Arc<Texture>, String>>,
    materials: HashMap<String, Result<Material, String>>,
    source_bytes: usize,
    custom_materials: usize,
}

#[derive(Clone)]
pub(super) struct Material {
    pub names: Vec<Option<String>>,
    pub fragment: Option<Arc<Fragment>>,
    pub blending: Blending,
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
        if let Some(cached) = resources.textures.get(&file) {
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
        resources.textures.insert(file, result);
        returned
    }

    pub(super) fn material(
        &self,
        name: &str,
        resources: &mut Resources,
    ) -> Result<Material, Error> {
        if !relative(name) {
            return Err(Error::Format(
                "material path must stay within its resource root".into(),
            ));
        }
        if let Some(cached) = resources.materials.get(name) {
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
        let result = self.read_material(name, resources).map_err(diagnostic);
        let returned = result
            .as_ref()
            .cloned()
            .map_err(|message| Error::Format(message.clone()));
        resources.materials.insert(name.into(), result);
        returned
    }

    fn read_material(&self, name: &str, resources: &mut Resources) -> Result<Material, Error> {
        let material = self.json(name)?;
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
                        | "textures"
                        | "combos"
                        | "constantshadervalues"
                        | "blending"
                        | "cullmode"
                        | "depthtest"
                        | "depthwrite"
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
        let combos = object(pass.get("combos"), 64, 64)?;
        let constants = object(pass.get("constantshadervalues"), 128, 128)?;
        let mut names = names(pass.get("textures"))?;
        // Wallpaper Engine's plain image shaders, in their versions, draw what the built-in
        // layer shader draws when nothing varies them.
        if matches!(shader, "genericimage" | "genericimage2") {
            if !combos.is_empty() || !constants.is_empty() {
                return Err(Error::Format(
                    "genericimage variants need another renderer".into(),
                ));
            }
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
            });
        }
        let fragment = self.custom(shader, &combos, &constants, &mut names, false, resources)?;
        Ok(Material {
            names,
            fragment: Some(fragment),
            blending,
        })
    }

    /// A custom shader's program for one pass. A `chained` pass is an effect's, whose slot 0
    /// is the layer's picture so far rather than a texture.
    pub(super) fn custom(
        &self,
        shader: &str,
        combos: &serde_json::Map<String, Value>,
        constants: &serde_json::Map<String, Value>,
        names: &mut Vec<Option<String>>,
        chained: bool,
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
        if chained && names.is_empty() {
            names.push(None);
        }
        template.fill_defaults(names, combos)?;
        let mask = if chained {
            names[0] = None;
            let mut mask = 1;
            for (slot, name) in names.iter().enumerate().skip(1) {
                if let Some(name) = name {
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
        let bytes = fragment.source.len() + fragment.vertex.as_ref().map_or(0, String::len);
        if bytes > (16 * 1024 * 1024usize).saturating_sub(resources.source_bytes) {
            return Err(Error::Format("scene shader sources exceed 16 MiB".into()));
        }
        resources.source_bytes += bytes;
        Ok(Arc::new(fragment))
    }
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
                texture_file(name)?;
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
