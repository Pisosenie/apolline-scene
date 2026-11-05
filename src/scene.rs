//! `scene.json`: the scene's size, camera and parallax, and its objects in drawing order.
//! Image layers and particle systems are read in full. Sounds and lights are kept by kind
//! only, so a host can tell what isn't drawn yet.

mod check;

use serde_json::Value;

pub use check::Missing;

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
    /// What in it isn't understood: keys, settings and changing values, read once.
    refused: Vec<Missing>,
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
    Particles(Particles),
    /// Nothing to draw: a place others hang from.
    Group,
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
    /// Multiplies its colour, above 1 to brighten it.
    pub brightness: f32,
    /// Where `origin` is on the layer, from its centre in shares of its size, y up.
    pub anchor: [f32; 2],
    pub visible: bool,
    /// How far it moves with the pointer, relative to the scene's parallax amount.
    pub depth: [f32; 2],
    /// The effects on it that are shown, as the scene gives them, in drawing order.
    pub effects: Vec<Value>,
}

/// A particle system placed in the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Particles {
    pub name: String,
    /// The system's own file, `particles/….json`.
    pub file: String,
    pub origin: [f32; 3],
    pub scale: [f32; 3],
    pub angles: [f32; 3],
    pub visible: bool,
    pub depth: [f32; 2],
    /// What this instance changes in the system: counts, rates, sizes and colour.
    pub overrides: serde_json::Map<String, Value>,
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
        let raw = root
            .get("objects")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let objects = hang(raw, raw.iter().map(object).collect());
        Ok(Scene {
            size: size.unwrap_or(DEFAULT_SIZE),
            perspective: size.is_none(),
            clear: vector(field("clearcolor"), [0.0; 3]),
            parallax,
            objects,
            refused: check::refusals(root),
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
    pub fn missing(&self) -> Vec<Missing> {
        let mut missing = Vec::new();
        let mut note = |what: Missing| {
            if !missing.contains(&what) {
                missing.push(what);
            }
        };
        if self.perspective {
            note(Missing::Camera3d);
        }
        for object in &self.objects {
            match object {
                Object::Image(image) if image.angles[0] != 0.0 || image.angles[1] != 0.0 => {
                    note(Missing::Rotation3d)
                }
                Object::Particles(particles)
                    if particles.angles[0] != 0.0 || particles.angles[1] != 0.0 =>
                {
                    note(Missing::Setting("particle systems tilted out of the scene"))
                }
                Object::Light => note(Missing::Lights),
                Object::Other => note(Missing::UnknownObject),
                _ => {}
            }
        }
        for refused in &self.refused {
            note(refused.clone());
        }
        missing
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
            brightness: number(field("brightness")).unwrap_or(1.0),
            anchor: anchor(text(field("alignment")).unwrap_or("center")),
            visible: flag(field("visible")).unwrap_or(true),
            depth: vector(field("parallaxDepth"), [1.0; 2]),
            effects: field("effects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|effect| flag(effect.get("visible")).unwrap_or(true))
                .take(64)
                .cloned()
                .collect(),
        });
    }
    if let Some(file) = text(field("particle")) {
        return Object::Particles(Particles {
            name: text(field("name")).unwrap_or_default().to_owned(),
            file: file.to_owned(),
            origin: vector(field("origin"), [0.0; 3]),
            scale: vector(field("scale"), [1.0; 3]),
            angles: vector(field("angles"), [0.0; 3]),
            visible: flag(field("visible")).unwrap_or(true),
            depth: vector(field("parallaxDepth"), [1.0; 2]),
            overrides: field("instanceoverride")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default(),
        });
    }
    if field("sound").is_some() {
        return Object::Sound;
    }
    if field("light").is_some() {
        return Object::Light;
    }
    let set = |key: &str| field(key).is_some_and(|value| !value.is_null());
    if !set("image") && !set("model") && !set("text") {
        return Object::Group;
    }
    Object::Other
}

/// Where an object sits on its own, before its parents move it.
struct Node {
    id: Option<i64>,
    parent: Option<i64>,
    origin: [f32; 3],
    scale: [f32; 3],
    angle: f32,
    visible: bool,
    /// Whether its parents' visibility reaches it.
    inherits: bool,
}

fn node(value: &Value) -> Node {
    let field = |name: &str| value.get(name);
    let id = |name: &str| field(name).and_then(Value::as_i64);
    Node {
        id: id("id"),
        parent: id("parent"),
        origin: vector(field("origin"), [0.0; 3]),
        scale: vector(field("scale"), [1.0; 3]),
        angle: vector(field("angles"), [0.0; 3])[2],
        visible: flag(field("visible")).unwrap_or(true),
        inherits: !flag(field("disablepropagation")).unwrap_or(false),
    }
}

/// Places each object inside its parents: a child's origin is in its parent's units, turned
/// and scaled with it, and a hidden parent hides it too.
fn hang(raw: &[Value], mut objects: Vec<Object>) -> Vec<Object> {
    let nodes: Vec<Node> = raw.iter().map(node).collect();
    let index: std::collections::HashMap<i64, usize> = nodes
        .iter()
        .enumerate()
        .filter_map(|(at, node)| Some((node.id?, at)))
        .collect();
    for (at, object) in objects.iter_mut().enumerate() {
        let mut origin = nodes[at].origin;
        let mut scale = nodes[at].scale;
        let mut angle = 0.0;
        let mut visible = true;
        let mut inherits = nodes[at].inherits;
        let mut parent = nodes[at].parent;
        // A bounded walk, so a loop of parents ends.
        for _ in 0..32 {
            let Some(&up) = parent.and_then(|id| index.get(&id)) else {
                break;
            };
            let above = &nodes[up];
            let (sin, cos) = above.angle.sin_cos();
            let local = [origin[0] * above.scale[0], origin[1] * above.scale[1]];
            origin = [
                above.origin[0] + cos * local[0] - sin * local[1],
                above.origin[1] + sin * local[0] + cos * local[1],
                above.origin[2] + origin[2] * above.scale[2],
            ];
            scale = [0, 1, 2].map(|axis| scale[axis] * above.scale[axis]);
            angle += above.angle;
            if inherits {
                visible &= above.visible;
            }
            inherits &= above.inherits;
            parent = above.parent;
        }
        match object {
            Object::Image(image) => {
                image.origin = origin;
                image.scale = scale;
                image.angles[2] += angle;
                image.visible &= visible;
            }
            Object::Particles(particles) => {
                particles.origin = origin;
                particles.scale = scale;
                particles.angles[2] += angle;
                particles.visible &= visible;
            }
            _ => {}
        }
    }
    objects
}

/// `topleft` and the rest as the point on the layer they name.
fn anchor(alignment: &str) -> [f32; 2] {
    let x = if alignment.contains("left") {
        -0.5
    } else if alignment.contains("right") {
        0.5
    } else {
        0.0
    };
    let y = if alignment.contains("top") {
        0.5
    } else if alignment.contains("bottom") {
        -0.5
    } else {
        0.0
    };
    [x, y]
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
        assert_eq!(images[1].effects.len(), 1);
        assert_eq!(scene.missing(), []);
    }

    #[test]
    fn a_scene_without_an_orthogonal_size_is_3d() {
        let scene = Scene::parse(r#"{"general": {"fov": 50}, "objects": []}"#).unwrap();
        assert!(scene.perspective);
        assert_eq!(scene.missing(), [Missing::Camera3d]);
        let tied = Scene::parse(
            r#"{"general": {"orthogonalprojection": {"width": 10, "height": 10}},
                "objects": [{"image": "a.json", "visible": {"user": {"name": "style", "condition": "2"}, "value": true}}]}"#,
        )
        .unwrap();
        assert_eq!(tied.missing(), [Missing::Conditional]);
        assert!(Scene::parse("not json").is_err());
    }

    #[test]
    fn children_sit_in_their_parents_turned_scaled_and_hidden_with_them() {
        let scene = Scene::parse(
            r#"{"general": {"orthogonalprojection": {"width": 400, "height": 400}}, "objects": [
                {"id": 1, "name": "group", "origin": "100 100 0", "scale": "2 2 1", "angles": "0 0 1.5707964"},
                {"id": 2, "parent": 1, "image": "a.json", "origin": "10 0 0", "scale": "0.5 1 1"},
                {"id": 3, "parent": 2, "image": "b.json", "origin": "0 0 0"},
                {"id": 4, "name": "hidden", "visible": false},
                {"id": 5, "parent": 4, "image": "c.json"},
                {"id": 6, "parent": 4, "image": "d.json", "disablepropagation": true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(scene.missing(), []);
        let images: Vec<&Image> = scene.images().collect();
        let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3);
        assert!(
            close(images[0].origin, [100.0, 120.0, 0.0]),
            "{:?}",
            images[0].origin
        );
        assert_eq!(images[0].scale, [1.0, 2.0, 1.0]);
        assert!((images[0].angles[2] - 1.5707964).abs() < 1e-6);
        assert!(close(images[1].origin, [100.0, 120.0, 0.0]));
        assert_eq!((images[2].visible, images[3].visible), (false, true));
    }
}
