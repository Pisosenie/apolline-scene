//! Typed user properties and their bindings. Conditions and scripts are kept as data;
//! loading a wallpaper never evaluates code from its metadata.

use crate::Error;
use serde_json::Value;
use std::collections::BTreeMap;

pub type Values = BTreeMap<String, Value>;

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Bool,
    Slider {
        min: f64,
        max: f64,
        fractional: bool,
    },
    Color,
    Combo(Vec<Choice>),
    Text,
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub label: String,
    pub value: Value,
}

impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub key: String,
    pub label: String,
    pub kind: Kind,
    pub default: Value,
    pub order: f64,
    pub condition: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    pub properties: Vec<Property>,
}

impl Schema {
    pub fn open(folder: &std::path::Path) -> Result<Self, Error> {
        let bytes = crate::files::read(&folder.join("project.json"), crate::files::JSON_BYTES)?;
        let project = serde_json::from_slice(&bytes)
            .map_err(|err| Error::Format(format!("project.json: {err}")))?;
        Self::parse(&project)
    }

    pub fn parse(project: &Value) -> Result<Self, Error> {
        let Some(properties) = project.pointer("/general/properties") else {
            return Ok(Self::default());
        };
        let properties = properties
            .as_object()
            .ok_or_else(|| Error::Format("general.properties must be an object".into()))?;
        if properties.len() > 512 {
            return Err(Error::Format("project exceeds 512 user properties".into()));
        }
        let mut parsed = Vec::new();
        for (key, value) in properties {
            let fail = |reason: &str| Error::Format(format!("property {key}: {reason}"));
            if key.is_empty() || key.len() > 128 {
                continue;
            }
            let kind = value
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            // A property described in a way it can't be edited stays read-only, and the scene
            // still loads with what it stores for it.
            let kind = (|| -> Result<Kind, Error> {
                Ok(match kind {
                    "bool" => Kind::Bool,
                    "color" => Kind::Color,
                    "textinput" => Kind::Text,
                    "slider" => {
                        let min = finite(value.get("min"))
                            .ok_or_else(|| fail("slider needs a finite minimum"))?;
                        let max = finite(value.get("max"))
                            .ok_or_else(|| fail("slider needs a finite maximum"))?;
                        if min >= max {
                            return Err(fail("slider minimum must be smaller than its maximum"));
                        }
                        Kind::Slider {
                            min,
                            max,
                            fractional: value
                                .get("fraction")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        }
                    }
                    "combo" => {
                        let options = value
                            .get("options")
                            .and_then(Value::as_array)
                            .ok_or_else(|| fail("combo needs options"))?;
                        if options.is_empty() || options.len() > 256 {
                            return Err(fail("combo needs 1–256 options"));
                        }
                        let mut choices = Vec::new();
                        for option in options {
                            let value = option
                                .get("value")
                                .filter(|v| v.is_string() || v.is_number() || v.is_boolean())
                                .ok_or_else(|| fail("invalid combo option value"))?
                                .clone();
                            if choices.iter().any(|choice: &Choice| choice.value == value) {
                                return Err(fail("duplicate combo option value"));
                            }
                            choices.push(Choice {
                                label: option
                                    .get("label")
                                    .or_else(|| option.get("text"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_owned(),
                                value,
                            });
                        }
                        Kind::Combo(choices)
                    }
                    other => Kind::Unsupported(other.to_owned()),
                })
            })()
            .unwrap_or_else(|err| Kind::Unsupported(err.to_string()));
            let property = Property {
                key: key.clone(),
                label: value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or(key)
                    .to_owned(),
                kind,
                default: value.get("value").cloned().unwrap_or(Value::Null),
                order: finite(value.get("order")).unwrap_or(0.0),
                condition: value
                    .get("condition")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            // A default the property can't hold leaves it read-only rather than refusing the
            // scene: what the scene itself stores for it still draws.
            let property = if matches!(property.kind, Kind::Unsupported(_))
                || property.validate(&property.default).is_ok()
            {
                property
            } else {
                Property {
                    kind: Kind::Unsupported("a default it can't hold".into()),
                    ..property
                }
            };
            parsed.push(property);
        }
        parsed.sort_by(|a, b| a.order.total_cmp(&b.order).then(a.key.cmp(&b.key)));
        Ok(Self { properties: parsed })
    }

    /// Each property's edit, or its default. Saved edits outlive the wallpaper they were made
    /// for: one for a property an update removed, or that no longer fits it, gives way to the
    /// default instead of keeping the scene from loading.
    pub fn resolve(&self, overrides: &Values) -> Result<Values, Error> {
        let mut values = Values::new();
        for property in &self.properties {
            if matches!(property.kind, Kind::Unsupported(_)) {
                continue;
            }
            let value = overrides
                .get(&property.key)
                .filter(|edit| property.validate(edit).is_ok())
                .unwrap_or(&property.default);
            values.insert(property.key.clone(), value.clone());
        }
        Ok(values)
    }
}

impl Property {
    fn validate(&self, value: &Value) -> Result<(), Error> {
        let valid = match &self.kind {
            Kind::Bool => value.is_boolean(),
            Kind::Slider {
                min,
                max,
                fractional,
            } => finite(Some(value))
                .is_some_and(|n| n >= *min && n <= *max && (*fractional || n.fract() == 0.0)),
            Kind::Color => value.as_str().is_some_and(|text| {
                let values: Vec<_> = text
                    .split_whitespace()
                    .take(4)
                    .map(str::parse::<f64>)
                    .collect();
                values.len() == 3
                    && values.iter().all(|n| {
                        n.as_ref()
                            .is_ok_and(|n| n.is_finite() && (0.0..=1.0).contains(n))
                    })
            }),
            Kind::Combo(choices) => choices.iter().any(|choice| &choice.value == value),
            Kind::Text => value.as_str().is_some_and(|text| text.len() <= 4096),
            Kind::Unsupported(_) => false,
        };
        if valid {
            Ok(())
        } else {
            Err(Error::Format(format!(
                "property {}: value does not match its type or allowed range",
                self.key
            )))
        }
    }
}

fn finite(value: Option<&Value>) -> Option<f64> {
    value?.as_f64().filter(|n| n.is_finite())
}

pub(crate) fn bind(root: &mut Value, values: &Values) {
    match root {
        Value::Object(fields) => {
            let user = fields.get("user");
            let bound = match user.and_then(condition) {
                // A flag that's whether the property is what the condition says.
                Some((key, wanted)) => values
                    .get(key)
                    .filter(|_| fields.get("value").is_some_and(Value::is_boolean))
                    .map(|value| Value::Bool(spelled(value) == wanted)),
                None => user
                    .and_then(Value::as_str)
                    .and_then(|key| values.get(key))
                    .cloned(),
            };
            if let Some(value) = bound
                && fields.contains_key("value")
            {
                fields.insert("value".into(), value);
            }
            for value in fields.values_mut() {
                bind(value, values);
            }
        }
        Value::Array(items) => {
            for value in items {
                bind(value, values);
            }
        }
        _ => {}
    }
}

/// A binding's property and the value it has to have: `{"name": …, "condition": …}`.
pub(crate) fn condition(user: &Value) -> Option<(&str, String)> {
    let name = user.get("name")?.as_str()?;
    Some((name, spelled(user.get("condition")?)))
}

/// A combo's value as text, however it was written: `"2"` and `2` alike.
fn spelled(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number
            .as_i64()
            .map_or_else(|| number.to_string(), |whole| whole.to_string()),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn project() -> Value {
        json!({"general":{"properties":{
            "show":{"type":"bool","value":true,"order":2},
            "alpha":{"type":"slider","min":0,"max":1,"fraction":true,"value":0.8,"order":1},
            "tint":{"type":"color","value":"1 0.5 0"},
            "style":{"type":"combo","value":"day","options":[{"value":"day","label":"Day"},{"value":"night","label":"Night"}]},
            "name":{"type":"textinput","value":"Hello"},
            "customfile":{"type":"file","value":""}
        }}})
    }
    #[test]
    fn a_condition_makes_a_flag_of_whether_a_combo_is_what_it_says() {
        let values = Values::from([("style".into(), json!("2")), ("count".into(), json!(3))]);
        let mut scene = json!({"objects": [
            {"visible": {"user": {"name": "style", "condition": "2"}, "value": false}},
            {"visible": {"user": {"name": "style", "condition": "1"}, "value": true}},
            {"visible": {"user": {"name": "count", "condition": "3"}, "value": false}},
            {"visible": {"user": {"name": "gone", "condition": "1"}, "value": true}}
        ]});
        bind(&mut scene, &values);
        let shown: Vec<&Value> = (0..4)
            .map(|at| &scene["objects"][at]["visible"]["value"])
            .collect();
        assert_eq!(
            shown,
            [&json!(true), &json!(false), &json!(true), &json!(true)]
        );
    }

    #[test]
    fn defaults_and_valid_edits_bind_without_executing_metadata() {
        let schema = Schema::parse(&project()).unwrap();
        let edits = Values::from([("show".into(), json!(false)), ("alpha".into(), json!(0.25))]);
        let values = schema.resolve(&edits).unwrap();
        assert_eq!(values["tint"], "1 0.5 0");
        assert!(!values.contains_key("customfile"));
        let mut scene = json!({"visible":{"user":"show","value":true},"alpha":{"user":"alpha","value":1},"script":"must remain data"});
        bind(&mut scene, &values);
        assert_eq!(scene["visible"]["value"], false);
        assert_eq!(scene["alpha"]["value"], 0.25);
        assert_eq!(scene["script"], "must remain data");
    }
    #[test]
    fn edits_that_dont_fit_give_way_to_defaults_and_bad_defaults_to_read_only() {
        let schema = Schema::parse(&project()).unwrap();
        let defaults = schema.resolve(&Values::new()).unwrap();
        for (key, value) in [
            ("show", json!("false")),
            ("alpha", json!(1.1)),
            ("tint", json!("NaN 0 1")),
            ("style", json!("unknown")),
            ("customfile", json!("/etc/passwd")),
            ("unknown", json!(true)),
        ] {
            let resolved = schema
                .resolve(&Values::from([(key.into(), value)]))
                .unwrap();
            assert_eq!(resolved, defaults, "{key}");
        }
        let mut invalid = project();
        invalid["general"]["properties"]["alpha"]["max"] = json!(0);
        let schema = Schema::parse(&invalid).unwrap();
        let resolved = schema
            .resolve(&Values::from([("alpha".into(), json!(0.5))]))
            .unwrap();
        assert!(!resolved.contains_key("alpha"));
    }
}
