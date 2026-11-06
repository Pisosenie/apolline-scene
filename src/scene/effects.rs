//! Layers' effects as the scene gives them, and the scripts on them: on whether each is
//! shown, and on the values its passes give their shaders.

use serde_json::Value;

use super::{Object, Script, Target, Text};
use crate::value::flag;

/// One of a layer's effects.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    /// Its place in the layer's list, which scripts find it by.
    pub index: usize,
    pub value: Value,
    /// Whether it's shown at first.
    pub visible: bool,
    /// Why it can't be drawn, for one hidden at first; scripts that show it fail.
    pub undrawn: Option<String>,
}

const MOST: usize = 64;

/// A layer's effects, shown or not, in drawing order.
pub(super) fn listed(value: Option<&Value>) -> Vec<Effect> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MOST)
        .enumerate()
        .map(|(index, effect)| Effect {
            index,
            value: effect.clone(),
            visible: flag(effect.get("visible")).unwrap_or(true),
            undrawn: None,
        })
        .collect()
}

/// Drops the hidden effects nothing can show: those without a script on whether they are,
/// in a scene whose scripts don't look effects up.
pub(super) fn keep(mut objects: Vec<Object>, looked_up: bool) -> Vec<Object> {
    for object in &mut objects {
        if let Object::Image(image) | Object::Text(Text { layer: image, .. }) = object {
            image.effects.retain(|effect| {
                effect.visible || looked_up || script(effect.value.get("visible")).is_some()
            });
        }
    }
    objects
}

/// Whether any script in the scene asks a layer for its effects.
pub(super) fn looked_up(raw: &[Value]) -> bool {
    fn asks(value: &Value) -> bool {
        match value {
            Value::Object(fields) => fields.iter().any(|(key, value)| {
                (key == "script"
                    && value
                        .as_str()
                        .is_some_and(|source| source.contains("getEffect")))
                    || asks(value)
            }),
            Value::Array(items) => items.iter().any(asks),
            _ => false,
        }
    }
    raw.iter().any(asks)
}

/// The scripts on an object's kept effects: on whether each is shown, then on its passes'
/// values, in the order the scene gives them.
pub(super) fn scripts(object: usize, objects: &[Object]) -> Vec<Script> {
    let effects = match objects.get(object) {
        Some(Object::Image(image) | Object::Text(Text { layer: image, .. })) => &image.effects,
        _ => return Vec::new(),
    };
    let mut found = Vec::new();
    for effect in effects {
        if let Some(fields) = script(effect.value.get("visible")) {
            found.push(made(object, Target::Shown(effect.index), fields));
        }
        let passes = effect.value.get("passes").and_then(Value::as_array);
        for (pass, over) in passes.into_iter().flatten().enumerate() {
            let values = over.get("constantshadervalues").and_then(Value::as_object);
            for (name, value) in values.into_iter().flatten() {
                if let Some(fields) = script(Some(value)) {
                    let target = Target::Value {
                        effect: effect.index,
                        pass,
                        name: name.clone(),
                    };
                    found.push(made(object, target, fields));
                }
            }
        }
    }
    found
}

fn script(value: Option<&Value>) -> Option<&serde_json::Map<String, Value>> {
    value
        .and_then(Value::as_object)
        .filter(|fields| fields.get("script").is_some_and(Value::is_string))
}

fn made(object: usize, target: Target, fields: &serde_json::Map<String, Value>) -> Script {
    Script {
        object,
        target,
        source: fields["script"].as_str().unwrap_or_default().to_owned(),
        properties: fields
            .get("scriptproperties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default(),
    }
}
