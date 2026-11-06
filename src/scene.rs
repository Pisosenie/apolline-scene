//! `scene.json`: the scene's size, camera and parallax, and its objects in drawing order.
//! Image layers, text layers and particle systems are read in full. Sounds and lights are kept
//! by kind only, so a host can tell what isn't drawn yet.

mod check;

use serde_json::Value;

pub use check::Missing;

use crate::Error;
use crate::value::{flag, number, plain, text, vector};

pub struct Scene {
    /// Its size in scene pixels, which layers are placed in, with y going up.
    pub size: (f32, f32),
    /// A perspective camera, for 3D scenes; only orthogonal ones are drawn so far.
    pub perspective: bool,
    pub clear: [f32; 3],
    pub parallax: Option<Parallax>,
    pub objects: Vec<Object>,
    /// Scripts on objects' places and looks, in the order they run.
    pub scripts: Vec<Script>,
    /// Each object's own place and look, before its parents move it, and what it hangs from.
    pub(crate) locals: Vec<Local>,
    pub(crate) tree: Vec<Node>,
    /// Every object's name, drawn or not, which scripts find layers by.
    pub(crate) names: Vec<String>,
    /// What in it isn't understood: keys, settings and changing values, read once.
    refused: Vec<Missing>,
}

/// A SceneScript module on one of an object's values.
#[derive(Clone, Debug, PartialEq)]
pub struct Script {
    pub object: usize,
    /// `origin`, `scale`, `angles`, `color`, `alpha`, `brightness`, `visible`, or a text
    /// layer's `text`.
    pub property: &'static str,
    pub source: String,
    /// The scene's values for the script's own properties, by name.
    pub properties: serde_json::Map<String, Value>,
}

/// The object values a script can be on.
pub(crate) const SCRIPTABLE: &[&str] = &[
    "origin",
    "scale",
    "angles",
    "color",
    "alpha",
    "brightness",
    "visible",
];

/// An object's own place and look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub origin: [f32; 3],
    pub scale: [f32; 3],
    /// In radians.
    pub angles: [f32; 3],
    pub color: [f32; 3],
    pub alpha: f32,
    pub brightness: f32,
    pub visible: bool,
}

/// Where an object ends up once its parents have moved it, and how it looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub origin: [f32; 3],
    pub scale: [f32; 3],
    /// In radians, in the scene's plane.
    pub angle: f32,
    pub color: [f32; 3],
    pub alpha: f32,
    pub brightness: f32,
    pub visible: bool,
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
    Text(Text),
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

impl Image {
    /// Where it is and how it looks, as the scene places it.
    pub fn pose(&self) -> Pose {
        Pose {
            origin: self.origin,
            scale: self.scale,
            angle: self.angles[2],
            color: self.color,
            alpha: self.alpha,
            brightness: self.brightness,
            visible: self.visible,
        }
    }
}

/// Words in a font, drawn into a picture that is then placed like an image layer's.
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    /// Where it is and how it looks. It has no model, and its size and anchor follow from
    /// its words.
    pub layer: Image,
    pub text: String,
    /// One of the project's font files, or `systemfont_…`.
    pub font: String,
    pub points: f32,
    /// Which side of the words `origin` is on, -1, 0 or 1: left to right, then bottom to top.
    pub align: [f32; 2],
    /// Clear room around the words, in scene pixels.
    pub padding: [f32; 2],
    /// The widest a line gets before it wraps, in scene pixels.
    pub width: Option<f32>,
    /// The most lines, and whether a cut ends in an ellipsis.
    pub rows: Option<usize>,
    pub ellipsis: bool,
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
        let objects: Vec<Object> = raw.iter().map(object).collect();
        let locals: Vec<Local> = raw.iter().map(local).collect();
        let tree = tree(raw);
        let objects = hang(objects, &pose(&locals, &tree));
        Ok(Scene {
            size: size.unwrap_or(DEFAULT_SIZE),
            perspective: size.is_none(),
            clear: vector(field("clearcolor"), [0.0; 3]),
            parallax,
            scripts: scripts(raw, &objects),
            objects,
            locals,
            tree,
            names: raw
                .iter()
                .map(|value| text(value.get("name")).unwrap_or_default().to_owned())
                .collect(),
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
                Object::Image(image) | Object::Text(Text { layer: image, .. })
                    if image.angles[0] != 0.0 || image.angles[1] != 0.0 =>
                {
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
        return Object::Image(picture(value, model));
    }
    if field("text").is_some_and(|words| !words.is_null()) {
        let side = |name: &str, low: &str, high: &str| match text(field(name)) {
            Some(side) if side == low => -1.0,
            Some(side) if side == high => 1.0,
            _ => 0.0,
        };
        let limit = |on: &str, value: &str| {
            flag(field(on))
                .unwrap_or(false)
                .then(|| number(field(value)))
                .flatten()
                .filter(|limit| *limit > 0.0)
        };
        return Object::Text(Text {
            layer: Image {
                size: None,
                anchor: [0.0; 2],
                ..picture(value, "")
            },
            text: text(field("text")).unwrap_or_default().to_owned(),
            font: text(field("font")).unwrap_or_default().to_owned(),
            points: number(field("pointsize")).unwrap_or(32.0),
            align: [
                side("horizontalalign", "left", "right"),
                side("verticalalign", "bottom", "top"),
            ],
            padding: match field("padding").map(plain) {
                Some(Value::Number(_)) => [number(field("padding")).unwrap_or(0.0); 2],
                _ => vector(field("padding"), [0.0; 2]),
            },
            width: limit("limitwidth", "maxwidth"),
            rows: limit("limitrows", "maxrows").map(|rows| rows as usize),
            ellipsis: flag(field("limituseellipsis")).unwrap_or(false),
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

/// An image layer's place, look and effects, as the scene gives them.
fn picture(value: &Value, model: &str) -> Image {
    let field = |name: &str| value.get(name);
    let size = vector(field("size"), [0.0; 2]);
    Image {
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
    }
}

fn local(value: &Value) -> Local {
    let field = |name: &str| value.get(name);
    Local {
        origin: vector(field("origin"), [0.0; 3]),
        scale: vector(field("scale"), [1.0; 3]),
        angles: vector(field("angles"), [0.0; 3]),
        color: vector(field("color"), [1.0; 3]),
        alpha: number(field("alpha")).unwrap_or(1.0),
        brightness: number(field("brightness")).unwrap_or(1.0),
        visible: flag(field("visible")).unwrap_or(true),
    }
}

/// What an object hangs from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Node {
    pub parent: Option<usize>,
    /// Whether its parents' visibility reaches it.
    pub inherits: bool,
}

fn tree(raw: &[Value]) -> Vec<Node> {
    let id = |value: &Value, name: &str| value.get(name).and_then(Value::as_i64);
    let index: std::collections::HashMap<i64, usize> = raw
        .iter()
        .enumerate()
        .filter_map(|(at, value)| Some((id(value, "id")?, at)))
        .collect();
    raw.iter()
        .map(|value| Node {
            parent: id(value, "parent").and_then(|parent| index.get(&parent).copied()),
            inherits: !flag(value.get("disablepropagation")).unwrap_or(false),
        })
        .collect()
}

/// Places each object inside its parents: a child's origin is in its parent's units, turned
/// and scaled with it, and a hidden parent hides it too.
pub(crate) fn pose(locals: &[Local], tree: &[Node]) -> Vec<Pose> {
    locals
        .iter()
        .zip(tree)
        .map(|(own, node)| {
            let mut origin = own.origin;
            let mut scale = own.scale;
            let mut angle = own.angles[2];
            let mut visible = own.visible;
            let mut inherits = node.inherits;
            let mut parent = node.parent;
            // A bounded walk, so a loop of parents ends.
            for _ in 0..32 {
                let Some(up) = parent.filter(|&up| up < locals.len()) else {
                    break;
                };
                let above = &locals[up];
                let (sin, cos) = above.angles[2].sin_cos();
                let local = [origin[0] * above.scale[0], origin[1] * above.scale[1]];
                origin = [
                    above.origin[0] + cos * local[0] - sin * local[1],
                    above.origin[1] + sin * local[0] + cos * local[1],
                    above.origin[2] + origin[2] * above.scale[2],
                ];
                scale = [0, 1, 2].map(|axis| scale[axis] * above.scale[axis]);
                angle += above.angles[2];
                if inherits {
                    visible &= above.visible;
                }
                inherits &= tree[up].inherits;
                parent = tree[up].parent;
            }
            Pose {
                origin,
                scale,
                angle,
                color: own.color,
                alpha: own.alpha,
                brightness: own.brightness,
                visible,
            }
        })
        .collect()
}

fn hang(mut objects: Vec<Object>, poses: &[Pose]) -> Vec<Object> {
    for (object, pose) in objects.iter_mut().zip(poses) {
        match object {
            Object::Image(image) | Object::Text(Text { layer: image, .. }) => {
                image.origin = pose.origin;
                image.scale = pose.scale;
                image.angles[2] = pose.angle;
                image.visible = pose.visible;
            }
            Object::Particles(particles) => {
                particles.origin = pose.origin;
                particles.scale = pose.scale;
                particles.angles[2] = pose.angle;
                particles.visible = pose.visible;
            }
            _ => {}
        }
    }
    objects
}

/// The scripts on drawn objects' values. Scripts anywhere else are refused by the checks.
fn scripts(raw: &[Value], objects: &[Object]) -> Vec<Script> {
    let mut found = Vec::new();
    for (at, (value, object)) in raw.iter().zip(objects).enumerate() {
        let words: &[&'static str] = match object {
            Object::Text(_) => &["text"],
            Object::Image(_) | Object::Particles(_) | Object::Group => &[],
            _ => continue,
        };
        for &property in SCRIPTABLE.iter().chain(words) {
            let Some(fields) = value.get(property).and_then(Value::as_object) else {
                continue;
            };
            let Some(source) = fields.get("script").and_then(Value::as_str) else {
                continue;
            };
            found.push(Script {
                object: at,
                property,
                source: source.to_owned(),
                properties: fields
                    .get("scriptproperties")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default(),
            });
        }
    }
    found
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

    #[test]
    fn text_layers_keep_their_words_font_and_limits() {
        let scene = Scene::parse(
            r#"{"general": {"orthogonalprojection": {"width": 400, "height": 400}}, "objects": [
                {"id": 1, "name": "clock", "font": "systemfont_arial", "origin": "10 20 0",
                 "text": {"script": "export function update() { return 'x'; }", "value": "12:00"},
                 "pointsize": {"user": "size", "value": 20}, "padding": 32, "alpha": 0.5,
                 "horizontalalign": "right", "verticalalign": "top", "anchor": "none",
                 "limitwidth": true, "maxwidth": 300, "limitrows": true, "maxrows": 2,
                 "limituseellipsis": true, "opaquebackground": false, "spacing": "0 0"},
                {"id": 2, "parent": 1, "name": "label", "text": "Hi", "font": "fonts/a.ttf",
                 "padding": "4.0 8.0", "limitwidth": false, "maxwidth": 500, "image": null}
            ]}"#,
        )
        .unwrap();
        assert_eq!(scene.missing(), []);
        let (Object::Text(clock), Object::Text(label)) = (&scene.objects[0], &scene.objects[1])
        else {
            panic!("text layers aren't read as text");
        };
        assert_eq!(
            (clock.text.as_str(), clock.font.as_str(), clock.points),
            ("12:00", "systemfont_arial", 20.0)
        );
        assert_eq!((clock.align, clock.padding), ([1.0, 1.0], [32.0; 2]));
        assert_eq!(
            (clock.width, clock.rows, clock.ellipsis),
            (Some(300.0), Some(2), true)
        );
        assert_eq!(
            (clock.layer.origin, clock.layer.alpha),
            ([10.0, 20.0, 0.0], 0.5)
        );
        assert_eq!((label.text.as_str(), label.points), ("Hi", 32.0));
        assert_eq!(
            (label.align, label.padding, label.width),
            ([0.0; 2], [4.0, 8.0], None)
        );
        // A text layer hangs from its parent like any other.
        assert_eq!(label.layer.origin, [10.0, 20.0, 0.0]);
        assert_eq!(scene.scripts.len(), 1);
        assert_eq!(
            (scene.scripts[0].object, scene.scripts[0].property),
            (0, "text")
        );
    }
}
