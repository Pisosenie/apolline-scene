//! A Wallpaper Engine project folder. `project.json` names the scene; the scene's files are in
//! `scene.pkg` or loose in the folder, and what neither has comes from Wallpaper Engine's own
//! `assets` folder, which holds the materials and shaders scenes share.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::Error;
use crate::pkg::Package;
use crate::scene::{Image, Scene};
use crate::tex::Texture;
use crate::value::{flag, text};

pub struct Project {
    folder: PathBuf,
    package: Option<Package>,
    assets: Option<PathBuf>,
    pub title: String,
    scene: String,
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
    pub texture: Texture,
    pub blending: Blending,
    /// It covers the whole scene, whatever its own origin and size say.
    pub fullscreen: bool,
}

impl Project {
    /// `assets` is Wallpaper Engine's `assets` folder, when it's on this machine.
    pub fn open(folder: &Path, assets: Option<&Path>) -> Result<Self, Error> {
        let read = |path: PathBuf| {
            std::fs::read_to_string(&path).map_err(|err| Error::Io(path.display().to_string(), err))
        };
        let project: Value = serde_json::from_str(&read(folder.join("project.json"))?)
            .map_err(|err| Error::Format(format!("project.json: {err}")))?;
        let kind = text(project.get("type")).unwrap_or("scene");
        if !kind.eq_ignore_ascii_case("scene") {
            return Err(Error::Format(format!("a {kind} wallpaper, not a scene")));
        }
        let package = folder.join("scene.pkg");
        let package = package
            .is_file()
            .then(|| Package::open(&package))
            .transpose()?;
        Ok(Project {
            folder: folder.to_path_buf(),
            package,
            assets: assets.map(Path::to_path_buf),
            title: text(project.get("title"))
                .unwrap_or_default()
                .trim()
                .to_owned(),
            scene: text(project.get("file")).unwrap_or("scene.json").to_owned(),
        })
    }

    /// A file by its path inside the project, from the package, the folder or the assets.
    pub fn file(&self, name: &str) -> Option<Cow<'_, [u8]>> {
        if name.split('/').any(|part| part == "..") {
            return None;
        }
        if let Some(bytes) = self.package.as_ref().and_then(|package| package.get(name)) {
            return Some(Cow::Borrowed(bytes));
        }
        [Some(&self.folder), self.assets.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|root| std::fs::read(root.join(name)).ok())
            .map(Cow::Owned)
    }

    fn json(&self, name: &str) -> Result<Value, Error> {
        let bytes = self
            .file(name)
            .ok_or_else(|| Error::Format(format!("{name} isn't in the project")))?;
        serde_json::from_slice(&bytes).map_err(|err| Error::Format(format!("{name}: {err}")))
    }

    pub fn scene(&self) -> Result<Scene, Error> {
        let bytes = self
            .file(&self.scene)
            .ok_or_else(|| Error::Format(format!("{} isn't in the project", self.scene)))?;
        Scene::parse(&String::from_utf8_lossy(&bytes))
    }

    /// The scene's image layers, each with its texture. A layer whose files are missing or
    /// broken is left out and named in the second list, so the rest still draws.
    pub fn layers(&self, scene: &Scene) -> (Vec<Layer>, Vec<String>) {
        let mut layers = Vec::new();
        let mut skipped = Vec::new();
        for image in scene.images() {
            match self.layer(image) {
                Ok(layer) => layers.push(layer),
                Err(err) => skipped.push(format!("{}: {err}", image.name)),
            }
        }
        (layers, skipped)
    }

    /// Model, then material, then the material's first pass and its first texture.
    fn layer(&self, image: &Image) -> Result<Layer, Error> {
        let model = self.json(&image.model)?;
        let material = text(model.get("material"))
            .ok_or_else(|| Error::Format(format!("{} names no material", image.model)))?;
        let material = self.json(material)?;
        let pass = material
            .get("passes")
            .and_then(Value::as_array)
            .and_then(|passes| passes.first())
            .ok_or_else(|| Error::Format("a material without passes".into()))?;
        let texture = pass
            .get("textures")
            .and_then(Value::as_array)
            .and_then(|textures| textures.first())
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Format("a material without a texture".into()))?;
        let file = format!("materials/{texture}.tex");
        let bytes = self
            .file(&file)
            .ok_or_else(|| Error::Format(format!("{file} isn't in the project or the assets")))?;
        Ok(Layer {
            image: image.clone(),
            texture: Texture::parse(&bytes)?,
            blending: match text(pass.get("blending")).unwrap_or("translucent") {
                "additive" => Blending::Additive,
                "normal" | "disabled" => Blending::Opaque,
                _ => Blending::Translucent,
            },
            fullscreen: flag(model.get("fullscreen")).unwrap_or(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pkg;

    /// A tiny RGBA texture in the format's second container version.
    fn texture(width: u32, height: u32) -> Vec<u8> {
        let mut out = b"TEXV0005\0TEXI0001\0".to_vec();
        for n in [0, 0, width, height, width, height, 0] {
            out.extend(u32::to_le_bytes(n));
        }
        out.extend(b"TEXB0002\0");
        let pixels = vec![200u8; (width * height * 4) as usize];
        for n in [1, 1, width, height, 0, 0, pixels.len() as u32] {
            out.extend(u32::to_le_bytes(n));
        }
        out.extend(pixels);
        out
    }

    fn folder(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("apolline-scene-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    #[test]
    fn layers_resolve_through_the_package_and_the_assets() {
        let folder = folder("project");
        let assets = folder.join("assets");
        std::fs::create_dir_all(assets.join("materials")).unwrap();
        std::fs::write(assets.join("materials/shared.tex"), texture(2, 2)).unwrap();
        std::fs::write(
            folder.join("project.json"),
            r#"{"title": " Rainy roof ", "type": "scene", "file": "scene.json"}"#,
        )
        .unwrap();
        let scene = br#"{"general": {"orthogonalprojection": {"width": 100, "height": 50}},
            "objects": [
                {"name": "roof", "image": "models/roof.json", "origin": "50 25 0"},
                {"name": "glow", "image": "models/glow.json"},
                {"name": "lost", "image": "models/lost.json"}
            ]}"#;
        let tex = texture(4, 2);
        std::fs::write(
            folder.join("scene.pkg"),
            pkg::build(&[
                ("scene.json", scene),
                (
                    "models/roof.json",
                    br#"{"material": "materials/roof.json", "fullscreen": true}"#,
                ),
                (
                    "materials/roof.json",
                    br#"{"passes": [{"textures": ["roof"], "blending": "normal"}]}"#,
                ),
                ("materials/roof.tex", &tex),
                (
                    "models/glow.json",
                    br#"{"material": "materials/glow.json"}"#,
                ),
                (
                    "materials/glow.json",
                    br#"{"passes": [{"textures": ["shared"], "blending": "additive"}]}"#,
                ),
            ]),
        )
        .unwrap();
        let project = Project::open(&folder, Some(&assets)).unwrap();
        assert_eq!(project.title, "Rainy roof");
        let scene = project.scene().unwrap();
        let (layers, skipped) = project.layers(&scene);
        assert_eq!(layers.len(), 2);
        assert_eq!(
            (layers[0].blending, layers[0].fullscreen),
            (Blending::Opaque, true)
        );
        assert_eq!(layers[0].texture.width, 4);
        assert_eq!(layers[1].blending, Blending::Additive);
        assert_eq!(layers[1].texture.width, 2);
        assert_eq!(skipped.len(), 1);
        assert!(skipped[0].starts_with("lost: "));
        assert!(project.file("../project.json").is_none());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn only_scenes_open() {
        let folder = folder("video");
        std::fs::write(
            folder.join("project.json"),
            r#"{"type": "video", "file": "a.mp4"}"#,
        )
        .unwrap();
        assert!(Project::open(&folder, None).is_err());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
