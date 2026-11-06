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
    Lights,
    UnknownObject,
    Conditional,
    /// A value that changes over time, at this key path.
    Animated(String),
    /// A script on a value scripts aren't run for yet, at this key path.
    Scripted(String),
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
            Missing::Lights => "lights",
            Missing::UnknownObject => "unknown-object",
            Missing::Conditional => "conditional",
            Missing::Animated(_) => "animated",
            Missing::Scripted(_) => "scripted",
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
            Missing::Lights => f.write_str("lights"),
            Missing::UnknownObject => f.write_str("unknown objects"),
            Missing::Conditional => f.write_str("values tied to property conditions"),
            Missing::Animated(path) => write!(f, "an animated value at {path}"),
            Missing::Scripted(path) => write!(f, "a script at {path}"),
            Missing::UnknownKey(path) => write!(f, "an unknown key {path}"),
            Missing::Setting(what) => f.write_str(what),
        }
    }
}

const ROOT: &[&str] = &["camera", "general", "objects", "version"];

/// `general` keys that are read, or that change nothing in a flat scene: the 3D lights'
/// colours, the editor's preview, the projection's depth range, and the HDR bloom, wind and
/// gravity settings while HDR, bloom and wind are off.
const GENERAL: &[&str] = &[
    "ambientcolor",
    "bloom",
    "bloomhdrfeather",
    "bloomhdriterations",
    "bloomhdrscatter",
    "bloomhdrstrength",
    "bloomhdrthreshold",
    "bloomstrength",
    "bloomthreshold",
    "bloomtint",
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
    "gravitydirection",
    "gravitystrength",
    "hdr",
    "nearz",
    "orthogonalprojection",
    "perspectiveoverridefov",
    "skylightcolor",
    "transparentsorting",
    "winddirection",
    "windenabled",
    "windstrength",
    "zoom",
];

/// Image layer keys that are read, or are the editor's own bookkeeping: `solid` makes a layer
/// catch the pointer, `ledsource` feeds RGB devices, `copybackground` matters only to shaders
/// that read the background, which are refused on their own.
const IMAGE: &[&str] = &[
    "alignment",
    "alpha",
    "angles",
    "brightness",
    "castshadow",
    "clampuvs",
    "color",
    "colorBlendMode",
    "copybackground",
    "dependencies",
    "depthtest",
    "disablepropagation",
    "effects",
    "id",
    "image",
    "ledsource",
    "locktransforms",
    "name",
    "origin",
    "parallaxDepth",
    "parent",
    "perspective",
    "scale",
    "size",
    "solid",
    "visible",
];

/// Where a layer's origin is on it, as alignments name it.
pub(crate) const ALIGNMENTS: &[&str] = &[
    "center",
    "left",
    "right",
    "top",
    "bottom",
    "topleft",
    "topright",
    "bottomleft",
    "bottomright",
];

/// Text layer keys that are read, or are the editor's own bookkeeping as on image layers:
/// `size` is the box the editor last measured, `anchor` keeps a layer by an edge of screens
/// shaped unlike the scene, and the background is drawn only when it's opaque.
const TEXT: &[&str] = &[
    "alpha",
    "anchor",
    "angles",
    "backgroundbrightness",
    "backgroundcolor",
    "blockalign",
    "brightness",
    "castshadow",
    "clampuvs",
    "color",
    "colorBlendMode",
    "copybackground",
    "dependencies",
    "depthtest",
    "disablepropagation",
    "effects",
    "font",
    "horizontalalign",
    "id",
    "image",
    "ledsource",
    "limitrows",
    "limituseellipsis",
    "limitwidth",
    "locktransforms",
    "maxrows",
    "maxwidth",
    "model",
    "name",
    "opaquebackground",
    "origin",
    "padding",
    "parallaxDepth",
    "parent",
    "particle",
    "perspective",
    "pointsize",
    "scale",
    "size",
    "solid",
    "spacing",
    "text",
    "verticalalign",
    "visible",
];

/// Particle system keys that are read, or are the editor's own bookkeeping.
const PARTICLES: &[&str] = &[
    "angles",
    "castshadow",
    "dependencies",
    "disablepropagation",
    "id",
    "image",
    "instanceoverride",
    "locktransforms",
    "model",
    "name",
    "origin",
    "parallaxDepth",
    "parent",
    "particle",
    "scale",
    "solid",
    "visible",
];

/// Group keys: a group only places what hangs from it.
const GROUP: &[&str] = &[
    "angles",
    "castshadow",
    "disablepropagation",
    "id",
    "image",
    "locktransforms",
    "model",
    "name",
    "origin",
    "parallaxDepth",
    "parent",
    "scale",
    "solid",
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
        if flag(field("hdr")) == Some(true) {
            note(Missing::Setting("HDR rendering"));
        }
        if flag(field("windenabled")) == Some(true) {
            note(Missing::Setting("wind"));
        }
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
        let set = |key: &str| fields.get(key).is_some_and(|value| !value.is_null());
        let lettered = !set("particle") && !set("image") && set("text");
        let known = if set("particle") {
            PARTICLES
        } else if set("image") {
            IMAGE
        } else if lettered {
            TEXT
        } else if !["model", "text", "sound", "light"]
            .iter()
            .any(|key| set(key))
        {
            GROUP
        } else {
            // Scripts only run on what's drawn; one anywhere else may move what is.
            for (key, value) in fields {
                if nested_script(value) {
                    note(Missing::Scripted(format!("objects[{at}].{key}")));
                }
            }
            continue;
        };
        // A key left null says nothing, like the editor's `particlesrc`.
        for (key, _) in fields
            .iter()
            .filter(|(key, value)| !known.contains(&key.as_str()) && !value.is_null())
        {
            note(Missing::UnknownKey(format!("objects[{at}].{key}")));
        }
        let field = |name: &str| fields.get(name);
        if text(field("alignment")).is_some_and(|alignment| !ALIGNMENTS.contains(&alignment)) {
            note(Missing::Setting("layers aligned in an unknown way"));
        }
        if number(field("colorBlendMode")).is_some_and(|mode| mode != 0.0) {
            note(Missing::Setting("colour blend modes"));
        }
        if flag(field("perspective")) == Some(true) {
            note(Missing::Setting("perspective layers"));
        }
        if flag(field("castshadow")) == Some(true) {
            note(Missing::Setting("layers that cast shadows"));
        }
        if fields.get("model").is_some_and(|model| !model.is_null()) {
            note(Missing::Setting("3D models"));
        }
        if lettered {
            words(fields, &mut note);
        }
        for (key, value) in fields {
            let scriptable =
                super::SCRIPTABLE.contains(&key.as_str()) || (lettered && key == "text");
            let animatable = super::ANIMATABLE.contains(&key.as_str());
            if key == "angles" && tilts(value) {
                note(Missing::Rotation3d);
                continue;
            }
            if known.contains(&key.as_str())
                && (scriptable && run(value)
                    || animatable && plays(value, scriptable) && fits(key, value))
            {
                continue;
            }
            let path = format!("objects[{at}].{key}");
            if key == "effects" && known.contains(&"effects") {
                effect_scripts(value, &path, &mut note);
            } else if value.get("animation").is_some() {
                note(Missing::Animated(path));
            } else if nested_script(value) {
                note(Missing::Scripted(path));
            } else {
                animated(value, &path, &mut note);
            }
        }
    }
    if let Some(general) = root.get("general").and_then(Value::as_object) {
        for (key, value) in general {
            let path = format!("general.{key}");
            if nested_script(value) {
                note(Missing::Scripted(path));
            } else {
                animated(value, &path, &mut note);
            }
        }
    }
    if root.get("camera").is_some_and(nested_script) {
        note(Missing::Scripted("camera".into()));
    }
    if conditional(root) {
        note(Missing::Conditional);
    }
    found
}

/// What a text layer asks for that isn't drawn yet.
fn words(fields: &serde_json::Map<String, Value>, note: &mut impl FnMut(Missing)) {
    let field = |name: &str| fields.get(name);
    let side =
        |name: &str, sides: &[&str]| text(field(name)).is_none_or(|side| sides.contains(&side));
    if !side("horizontalalign", &["left", "center", "right"])
        || !side("verticalalign", &["top", "center", "bottom"])
    {
        note(Missing::Setting("text aligned in an unknown way"));
    }
    let anchors = ["none", "center", "left", "right", "top", "bottom"];
    if text(field("anchor")).is_some_and(|anchor| !anchors.contains(&anchor)) {
        note(Missing::Setting("text anchored in an unknown way"));
    }
    if text(field("font")).is_none_or(str::is_empty) {
        note(Missing::Setting("text without a font"));
    }
    if flag(field("opaquebackground")) == Some(true) {
        note(Missing::Setting("text on a background"));
    }
    if flag(field("blockalign")) == Some(true) {
        note(Missing::Setting("text aligned as a block"));
    }
    if crate::value::vector(field("spacing"), [0.0; 2]) != [0.0; 2] {
        note(Missing::Setting("spaced-out text"));
    }
}

/// A value object with more than `user` and `value` in it: an animation, a script or
/// something else that changes it, which a stored value can't stand for.
fn animated(value: &Value, path: &str, note: &mut impl FnMut(Missing)) {
    if let Value::Object(fields) = value
        && fields.contains_key("value")
        && fields.keys().any(|key| key != "value" && key != "user")
    {
        note(if fields.contains_key("script") {
            Missing::Scripted(path.to_owned())
        } else {
            Missing::Animated(path.to_owned())
        });
    }
}

/// Scripts in a layer's effects that don't run: those anywhere but on whether an effect is
/// shown and on the values its passes give their shaders. A value whose script plays an
/// animation is an animated one.
fn effect_scripts(effects: &Value, path: &str, note: &mut impl FnMut(Missing)) {
    let scripted = || Missing::Scripted(path.to_owned());
    let mut found = Vec::new();
    let stray = |value: &Value, found: &mut Vec<Missing>| {
        if nested_script(value) {
            found.push(scripted());
        }
    };
    fn passes(effect: &Value) -> Option<&Vec<Value>> {
        effect.get("passes").and_then(Value::as_array)
    }
    for effect in effects.as_array().into_iter().flatten() {
        let Some(fields) = effect.as_object() else {
            stray(effect, &mut found);
            continue;
        };
        for (key, value) in fields {
            if value.get("animation").is_some() {
                found.push(Missing::Animated(path.to_owned()));
            } else if !(key == "visible" && run(value)
                || key == "passes" && passes(effect).is_some())
            {
                stray(value, &mut found);
            }
        }
        for pass in passes(effect).into_iter().flatten() {
            let Some(pass) = pass.as_object() else {
                stray(pass, &mut found);
                continue;
            };
            for (key, value) in pass {
                let Some(values) = value.as_object().filter(|_| key == "constantshadervalues")
                else {
                    stray(value, &mut found);
                    continue;
                };
                for value in values
                    .values()
                    .filter(|value| !run(value) && !plays(value, true))
                {
                    if value.get("animation").is_some() {
                        found.push(Missing::Animated(path.to_owned()));
                    } else {
                        stray(value, &mut found);
                    }
                }
            }
        }
    }
    if !effects.is_array() {
        stray(effects, &mut found);
    }
    found.into_iter().for_each(note);
}

/// A script anywhere in a value: on an effect's values, a particle override, anything the
/// scene's scripts don't run.
fn nested_script(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields.get("script").is_some_and(Value::is_string) || fields.values().any(nested_script)
        }
        Value::Array(items) => items.iter().any(nested_script),
        _ => false,
    }
}

/// A value with a script on it and nothing else that changes it, which the scene's scripts run.
fn run(value: &Value) -> bool {
    value.as_object().is_some_and(|fields| {
        fields.get("script").is_some_and(Value::is_string)
            && fields.keys().all(|key| {
                matches!(
                    key.as_str(),
                    "script" | "scriptproperties" | "value" | "user"
                )
            })
    })
}

/// A value a timeline plays, which it can, with nothing else in it but a script where one runs.
fn plays(value: &Value, scriptable: bool) -> bool {
    value.as_object().is_some_and(|fields| {
        fields
            .get("animation")
            .is_some_and(|animation| super::Timeline::parse(animation).is_ok())
            && fields.get("script").is_none_or(Value::is_string)
            && fields.keys().all(|key| {
                matches!(key.as_str(), "animation" | "value" | "user")
                    || scriptable && matches!(key.as_str(), "script" | "scriptproperties")
            })
    })
}

/// Whether a timeline only has keys for components the value has: one for `alpha` and
/// `brightness`, three for the rest.
fn fits(key: &str, value: &Value) -> bool {
    let width = if matches!(key, "alpha" | "brightness") {
        1
    } else {
        3
    };
    value
        .get("animation")
        .and_then(Value::as_object)
        .is_some_and(|fields| {
            fields
                .keys()
                .filter_map(|key| super::timeline::channel(key))
                .all(|channel| channel < width)
        })
}

/// A timeline that turns a layer about x or y, out of the scene's plane.
fn tilts(value: &Value) -> bool {
    let animation = value.get("animation");
    ["c0", "c1"].iter().any(|channel| {
        animation
            .and_then(|animation| animation.get(*channel))
            .and_then(Value::as_array)
            .is_some_and(|keys| {
                keys.iter()
                    .any(|key| number(key.get("value")).is_some_and(|turn| turn != 0.0))
            })
    })
}

/// `{"user": {"name": …, "condition": …}}` anywhere that isn't a flag set by whether a
/// property is what the condition says, which is what's drawn.
fn conditional(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            fields.get("user").is_some_and(|user| {
                user.is_object()
                    && (crate::properties::condition(user).is_none()
                        || !fields.get("value").is_some_and(Value::is_boolean))
            }) || fields.values().any(conditional)
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
            "objects": [{"image": "a.json", "wobble": 3, "colorBlendMode": 9,
                         "origin": {"value": "0 0 0", "animation": {"frames": []}}}]
        });
        let found = refusals(&scene);
        for missing in [
            Missing::UnknownKey("general.mystery".into()),
            Missing::Setting("the scene's bloom"),
            Missing::Setting("camera zoom"),
            Missing::UnknownKey("objects[0].wobble".into()),
            Missing::Setting("colour blend modes"),
            Missing::Animated("objects[0].origin".into()),
        ] {
            assert!(found.contains(&missing), "{missing} not in {found:?}");
        }
        assert_eq!(Missing::Animated("x".into()).code(), "animated");
    }

    #[test]
    fn text_layers_refuse_what_their_words_dont_draw() {
        let scene = json!({"objects": [
            {"text": {"script": "export function update(v) { return v; }", "value": "a"},
             "font": "fonts/a.ttf", "opaquebackground": true, "horizontalalign": "justify",
             "spacing": "2 0", "blockalign": true, "wobble": 1, "anchor": "middle"},
            {"text": "b", "font": "", "maxwidth": {"value": 10, "animation": {"frames": []}}, "particlesrc": null}
        ]});
        assert_eq!(
            refusals(&scene),
            [
                Missing::UnknownKey("objects[0].wobble".into()),
                Missing::Setting("text aligned in an unknown way"),
                Missing::Setting("text anchored in an unknown way"),
                Missing::Setting("text on a background"),
                Missing::Setting("text aligned as a block"),
                Missing::Setting("spaced-out text"),
                Missing::Setting("text without a font"),
                Missing::Animated("objects[1].maxwidth".into()),
            ]
        );
    }

    #[test]
    fn timelines_play_on_places_looks_and_effect_values() {
        let keys = json!([{"frame": 0, "value": 0}, {"frame": 10, "value": 1}]);
        let plays =
            json!({"value": 1, "animation": {"c0": keys, "options": {"fps": 30, "length": 10}}});
        let broken = json!({"value": 1, "animation": {"c0": keys, "options": {"fps": 30}}});
        let scene = json!({"objects": [
            {"image": "a.json", "alpha": plays, "origin": broken,
             "effects": [{"file": "e.json", "passes": [{"constantshadervalues": {"speed": plays}}]}]},
            {"image": "b.json", "visible": plays},
            {"text": "x", "font": "f.ttf", "maxwidth": plays},
            {"image": "c.json", "angles": {"value": "0 0 0", "animation": {"c1": keys, "options": {"fps": 30, "length": 10}}}},
            {"image": "d.json", "alpha": {"value": 1, "animation": {"c1": keys, "options": {"fps": 30, "length": 10}}},
             "effects": [{"file": "e.json", "visible": {"value": true, "animation": {"c0": keys, "options": {"fps": 30, "length": 10}}}}]}
        ]});
        assert_eq!(
            refusals(&scene),
            [
                Missing::Animated("objects[0].origin".into()),
                Missing::Animated("objects[1].visible".into()),
                Missing::Animated("objects[2].maxwidth".into()),
                Missing::Rotation3d,
                Missing::Animated("objects[4].alpha".into()),
                Missing::Animated("objects[4].effects".into()),
            ]
        );
    }

    #[test]
    fn scripts_run_only_on_drawn_objects_own_values() {
        let script = json!({"script": "export function update(v) { return v; }", "value": 1});
        let scene = json!({
            "general": {"zoom": script},
            "objects": [
                {"image": "a.json", "alpha": script, "visible": script,
                 "effects": [{"file": "e.json", "visible": script,
                              "passes": [{"constantshadervalues": {"speed": script}}]}]},
                {"image": "b.json", "effects": [{"file": "e.json", "passes": [{"combos": {"A": script}}]}]},
                {"image": "c.json", "effects": [{"file": "e.json", "passes": [{"constantshadervalues":
                    {"speed": {"script": "export function update() {}", "value": 1, "animation": {}}}}]}]},
                {"particle": "p.json", "instanceoverride": {"rate": script}},
                {"name": "group", "origin": script},
                {"sound": ["a.mp3"], "volume": script}
            ]
        });
        assert_eq!(
            refusals(&scene),
            [
                Missing::Scripted("objects[1].effects".into()),
                Missing::Animated("objects[2].effects".into()),
                Missing::Scripted("objects[3].instanceoverride".into()),
                Missing::Scripted("objects[5].volume".into()),
                Missing::Scripted("general.zoom".into()),
            ]
        );
    }
}
