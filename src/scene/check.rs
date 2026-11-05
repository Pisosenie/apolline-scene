//! What in a scene isn't understood, so it can be refused rather than drawn wrong: keys no
//! one has checked against real files, flags that change the picture and aren't drawn yet,
//! and values that change over time. Which keys are harmless is a guess until the survey of
//! real installs confirms it; until then the guess errs on the side of refusing.

use std::fmt;

use serde_json::Value;

use crate::value::{flag, number, text};

/// Why a scene isn't drawn here, in a form that can be counted across many scenes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    Camera3d,
    Rotation3d,
    PointerParallax,
    Particles,
    Lights,
    UnknownObject,
    Conditional,
    /// A value that changes over time or is computed, at this key path.
    Animated(String),
    /// A key no one has checked, at this path.
    UnknownKey(String),
    /// A setting that changes the picture and isn't drawn yet.
    Setting(&'static str),
}

impl Missing {
    /// A short stable name, for counting.
    pub fn code(&self) -> &'static str {
        match self {
            Missing::Camera3d => "camera3d",
            Missing::Rotation3d => "rotation3d",
            Missing::PointerParallax => "pointer-parallax",
            Missing::Particles => "particles",
            Missing::Lights => "lights",
            Missing::UnknownObject => "unknown-object",
            Missing::Conditional => "conditional",
            Missing::Animated(_) => "animated",
            Missing::UnknownKey(_) => "unknown-key",
            Missing::Setting(_) => "setting",
        }
    }
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Missing::Camera3d => f.write_str("a 3D camera"),
            Missing::Rotation3d => f.write_str("3D layer rotation"),
            Missing::PointerParallax => f.write_str("pointer parallax"),
            Missing::Particles => f.write_str("particles"),
            Missing::Lights => f.write_str("lights"),
            Missing::UnknownObject => f.write_str("unknown objects"),
            Missing::Conditional => f.write_str("values tied to property conditions"),
            Missing::Animated(path) => write!(f, "an animated or scripted value at {path}"),
            Missing::UnknownKey(path) => write!(f, "an unknown key {path}"),
            Missing::Setting(what) => f.write_str(what),
        }
    }
}

const ROOT: &[&str] = &["camera", "general", "objects", "version"];

/// `general` keys that are read, or that change nothing in a flat scene: the 3D lights'
/// colours, the editor's preview and the projection's depth range.
const GENERAL: &[&str] = &[
    "ambientcolor",
    "bloom",
    "bloomstrength",
    "bloomthreshold",
    "camerafade",
    "cameraparallax",
    "cameraparallaxamount",
    "cameraparallaxdelay",
    "cameraparallaxmouseinfluence",
    "camerapreview",
    "camerashake",
    "camerashakeamplitude",
    "camerashakeroughness",
    "camerashakespeed",
    "clearcolor",
    "clearenabled",
    "farz",
    "fov",
    "nearz",
    "orthogonalprojection",
    "skylightcolor",
    "zoom",
];

/// Image layer keys that are read, or are the editor's own bookkeeping.
const IMAGE: &[&str] = &[
    "alignment",
    "alpha",
    "angles",
    "brightness",
    "color",
    "colorBlendMode",
    "copybackground",
    "effects",
    "id",
    "image",
    "locktransforms",
    "name",
    "origin",
    "parallaxDepth",
    "perspective",
    "scale",
    "size",
    "visible",
];

/// Everything in a scene that isn't understood, each once.
pub(super) fn refusals(root: &Value) -> Vec<Missing> {
    let mut found = Vec::new();
    let mut note = |missing: Missing| {
        if found.len() < 32 && !found.contains(&missing) {
            found.push(missing);
        }
    };
    if let Some(fields) = root.as_object() {
        for key in fields.keys().filter(|key| !ROOT.contains(&key.as_str())) {
            note(Missing::UnknownKey(format!("scene.{key}")));
        }
    }
    if let Some(general) = root.get("general").and_then(Value::as_object) {
        for key in general
            .keys()
            .filter(|key| !GENERAL.contains(&key.as_str()))
        {
            note(Missing::UnknownKey(format!("general.{key}")));
        }
        let field = |name: &str| general.get(name);
        if flag(field("bloom")) == Some(true) {
            note(Missing::Setting("the scene's bloom"));
        }
        if flag(field("camerashake")) == Some(true) {
            note(Missing::Setting("camera shake"));
        }
        if flag(field("clearenabled")) == Some(false) {
            note(Missing::Setting("a scene that isn't cleared"));
        }
        if number(field("zoom")).is_some_and(|zoom| (zoom - 1.0).abs() > 1e-4) {
            note(Missing::Setting("camera zoom"));
        }
    }
    let objects = root.get("objects").and_then(Value::as_array);
    for (at, object) in objects.into_iter().flatten().enumerate() {
        let Some(fields) = object.as_object() else {
            continue;
        };
        if !fields.contains_key("image") {
            continue;
        }
        for key in fields.keys().filter(|key| !IMAGE.contains(&key.as_str())) {
            note(Missing::UnknownKey(format!("objects[{at}].{key}")));
        }
        let field = |name: &str| fields.get(name);
        if text(field("alignment")).is_some_and(|alignment| alignment != "center") {
            note(Missing::Setting(
                "layers aligned other than by their centre",
            ));
        }
        if number(field("colorBlendMode")).is_some_and(|mode| mode != 0.0) {
            note(Missing::Setting("colour blend modes"));
        }
        if flag(field("copybackground")) == Some(true) {
            note(Missing::Setting("layers that copy the background"));
        }
        if number(field("brightness")).is_some_and(|brightness| (brightness - 1.0).abs() > 1e-4) {
            note(Missing::Setting("layer brightness"));
        }
        if flag(field("perspective")) == Some(true) {
            note(Missing::Setting("perspective layers"));
        }
        for (key, value) in fields {
            animated(value, &format!("objects[{at}].{key}"), &mut note);
        }
    }
    if let Some(general) = root.get("general").and_then(Value::as_object) {
        for (key, value) in general {
            animated(value, &format!("general.{key}"), &mut note);
        }
    }
    if conditional(root) {
        note(Missing::Conditional);
    }
    found
}

/// A value object with more than `user` and `value` in it: an animation, a script or
/// something else that changes it, which a stored value can't stand for.
fn animated(value: &Value, path: &str, note: &mut impl FnMut(Missing)) {
    if let Value::Object(fields) = value
        && fields.contains_key("value")
        && fields.keys().any(|key| key != "value" && key != "user")
    {
        note(Missing::Animated(path.to_owned()));
    }
}

/// `{"user": {"name": …, "condition": …}}` anywhere: a value that follows a property only
/// under a condition.
fn conditional(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields.get("user").is_some_and(Value::is_object) || fields.values().any(conditional)
        }
        Value::Array(items) => items.iter().any(conditional),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_plain_scene_has_nothing_to_refuse() {
        let scene = json!({
            "camera": {}, "version": 3,
            "general": {"clearcolor": "0 0 0", "orthogonalprojection": {"width": 10, "height": 10},
                        "bloom": false, "camerashake": false, "zoom": 1, "ambientcolor": "0.3 0.3 0.3"},
            "objects": [{"id": 1, "name": "sky", "image": "models/sky.json", "origin": "5 5 0",
                         "alignment": "center", "colorBlendMode": 0, "copybackground": false,
                         "visible": {"user": "show", "value": true}}]
        });
        assert_eq!(refusals(&scene), []);
    }

    #[test]
    fn what_isnt_understood_is_named() {
        let scene = json!({
            "general": {"bloom": true, "mystery": 1, "zoom": 2},
            "objects": [{"image": "a.json", "parent": 3, "colorBlendMode": 9,
                         "origin": {"value": "0 0 0", "animation": {"frames": []}}}]
        });
        let found = refusals(&scene);
        for missing in [
            Missing::UnknownKey("general.mystery".into()),
            Missing::Setting("the scene's bloom"),
            Missing::Setting("camera zoom"),
            Missing::UnknownKey("objects[0].parent".into()),
            Missing::Setting("colour blend modes"),
            Missing::Animated("objects[0].origin".into()),
        ] {
            assert!(found.contains(&missing), "{missing} not in {found:?}");
        }
        assert_eq!(Missing::Animated("x".into()).code(), "animated");
    }
}
