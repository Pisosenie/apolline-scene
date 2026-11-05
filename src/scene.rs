//! `scene.json`: the scene's size, camera and parallax, and its objects in drawing order.
//! Image layers are read in full. Particles, sounds and lights are kept by kind only, so a
//! host can tell what isn't drawn yet.

use serde_json::Value;

use crate::Error;
use crate::value::{flag, number, text, vector};

pub struct Scene {
    /// Its size in scene pixels, which layers are placed in, with y going up.
    pub size: (f32, f32),
    /// A perspective camera, for 3D scenes; only orthogonal ones are drawn so far.
    pub perspective: bool,
    pub clear: [f32; 3],
    pub parallax: Option<Parallax>,
    pub objects: Vec<Object>,
    /// Some value follows a property only under a condition, `{"user": {"name", "condition"}}`,
    /// which isn't drawn yet: its stored value could show the wrong layers.
    pub conditional: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parallax {
    pub amount: f32,
    /// How slowly layers follow the pointer, in seconds.
    pub delay: f32,
    pub mouse: f32,
}

pub enum Object {
    Image(Image),
    Particles(String),
    Sound,
    Light,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub name: String,
    /// The model file, `models/….json`, which names the material and so the texture.
    pub model: String,
    pub origin: [f32; 3],
    pub scale: [f32; 3],
    /// In radians; z turns the layer in the scene's plane.
    pub angles: [f32; 3],
    /// Its size in scene pixels, when the scene gives one; otherwise the texture's.
    pub size: Option<(f32, f32)>,
    pub color: [f32; 3],
    pub alpha: f32,
    pub visible: bool,
    /// How far it moves with the pointer, relative to the scene's parallax amount.
    pub depth: [f32; 2],
    /// Effects on it, drawn from stage 3 on.
    pub effects: usize,
}

const DEFAULT_SIZE: (f32, f32) = (1920.0, 1080.0);

impl Scene {
    pub fn parse(json: &str) -> Result<Self, Error> {
        let root: Value = serde_json::from_str(json)
            .map_err(|err| Error::Format(format!("scene.json: {err}")))?;
        Self::from_value(&root)
    }

    pub(crate) fn from_value(root: &Value) -> Result<Self, Error> {
        if !root.is_object() {
            return Err(Error::Format("scene.json must contain an object".into()));
        }
        let general = root.get("general");
        let field = |name: &str| general.and_then(|general| general.get(name));
        let projection = field("orthogonalprojection").filter(|p| p.is_object());
        let size = projection
            .and_then(|p| Some((number(p.get("width"))?, number(p.get("height"))?)))
            .filter(|&(w, h)| w > 0.0 && h > 0.0);
        let parallax = flag(field("cameraparallax"))
            .unwrap_or(false)
            .then(|| Parallax {
                amount: number(field("cameraparallaxamount")).unwrap_or(0.5),
                delay: number(field("cameraparallaxdelay")).unwrap_or(0.1),
                mouse: number(field("cameraparallaxmouseinfluence")).unwrap_or(0.5),
            });
        let objects = root
            .get("objects")
            .and_then(Value::as_array)
            .map(|objects| objects.iter().map(object).collect())
            .unwrap_or_default();
        Ok(Scene {
            size: size.unwrap_or(DEFAULT_SIZE),
            perspective: size.is_none(),
            clear: vector(field("clearcolor"), [0.0; 3]),
            parallax,
            objects,
            conditional: conditional(root),
        })
    }

    pub fn images(&self) -> impl Iterator<Item = &Image> {
        self.objects.iter().filter_map(|object| match object {
            Object::Image(image) => Some(image),
            _ => None,
        })
    }

    /// What the scene has that isn't drawn yet, for a host deciding whether to hand it to
    /// another renderer.
    pub fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        let mut note = |what: &'static str| {
            if !missing.contains(&what) {
                missing.push(what);
            }
        };
        if self.perspective {
            note("a 3D camera");
        }
        if self.conditional {
            note("values tied to property conditions");
        }
        for object in &self.objects {
            match object {
                Object::Image(image) if image.effects > 0 => note("effects"),
                Object::Particles(_) => note("particles"),
                Object::Light => note("lights"),
                Object::Other => note("unknown objects"),
                _ => {}
            }
        }
        missing
    }
}

fn conditional(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields.get("user").is_some_and(Value::is_object) || fields.values().any(conditional)
        }
        Value::Array(items) => items.iter().any(conditional),
        _ => false,
    }
}

fn object(value: &Value) -> Object {
    let field = |name: &str| value.get(name);
    if let Some(model) = text(field("image")) {
        let size = vector(field("size"), [0.0; 2]);
        return Object::Image(Image {
            name: text(field("name")).unwrap_or_default().to_owned(),
            model: model.to_owned(),
            origin: vector(field("origin"), [0.0; 3]),
            scale: vector(field("scale"), [1.0; 3]),
            angles: vector(field("angles"), [0.0; 3]),
            size: (size[0] > 0.0 && size[1] > 0.0).then_some((size[0], size[1])),
            color: vector(field("color"), [1.0; 3]),
            alpha: number(field("alpha")).unwrap_or(1.0),
            visible: flag(field("visible")).unwrap_or(true),
            depth: vector(field("parallaxDepth"), [1.0; 2]),
            effects: field("effects")
                .and_then(Value::as_array)
                .map_or(0, |effects| {
                    effects
                        .iter()
                        .filter(|effect| flag(effect.get("visible")).unwrap_or(true))
                        .count()
                }),
        });
    }
    if let Some(particles) = text(field("particle")) {
        return Object::Particles(particles.to_owned());
    }
    if field("sound").is_some() {
        return Object::Sound;
    }
    if field("light").is_some() {
        return Object::Light;
    }
    Object::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r#"{
        "camera": {"center": "0 0 -1", "eye": "0 0 0", "up": "0 1 0"},
        "general": {
            "clearcolor": "0.1 0.2 0.3",
            "cameraparallax": true, "cameraparallaxamount": 0.8,
            "orthogonalprojection": {"width": 2560, "height": 1440}
        },
        "objects": [
            {"name": "sky", "image": "models/sky.json", "origin": "1280 720 0",
             "size": "2560 1440", "parallaxDepth": "0.2 0.2"},
            {"name": "clouds", "image": "models/clouds.json", "origin": "1280 1100 0",
             "scale": "1.5 1.5 1", "angles": "0 0 0.1", "alpha": {"user": "clouds", "value": 0.6},
             "visible": {"user": "show_clouds", "value": false},
             "effects": [{"file": "effects/waterripple/effect.json"}, {"file": "x", "visible": false}]},
            {"particle": "particles/rain.json"},
            {"sound": ["sounds/rain.mp3"]}
        ],
        "version": 3
    }"#;

    #[test]
    fn layers_come_out_with_their_place_look_and_depth() {
        let scene = Scene::parse(SCENE).unwrap();
        assert_eq!(scene.size, (2560.0, 1440.0));
        assert!(!scene.perspective);
        assert_eq!(scene.clear, [0.1, 0.2, 0.3]);
        assert_eq!(scene.parallax.map(|p| p.amount), Some(0.8));
        let images: Vec<&Image> = scene.images().collect();
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].size, Some((2560.0, 1440.0)));
        assert_eq!(images[0].depth, [0.2, 0.2]);
        assert_eq!(images[1].size, None);
        assert_eq!(images[1].scale, [1.5, 1.5, 1.0]);
        assert_eq!((images[1].alpha, images[1].visible), (0.6, false));
        assert_eq!(images[1].effects, 1);
        assert_eq!(scene.missing(), ["effects", "particles"]);
    }

    #[test]
    fn a_scene_without_an_orthogonal_size_is_3d() {
        let scene = Scene::parse(r#"{"general": {"fov": 50}, "objects": []}"#).unwrap();
        assert!(scene.perspective);
        assert_eq!(scene.missing(), ["a 3D camera"]);
        let tied = Scene::parse(
            r#"{"general": {"orthogonalprojection": {"width": 10, "height": 10}},
                "objects": [{"image": "a.json", "visible": {"user": {"name": "style", "condition": "2"}, "value": true}}]}"#,
        )
        .unwrap();
        assert_eq!(tied.missing(), ["values tied to property conditions"]);
        assert!(Scene::parse("not json").is_err());
    }
}
