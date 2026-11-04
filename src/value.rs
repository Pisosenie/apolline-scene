//! Values the way Wallpaper Engine's JSON writes them: vectors as strings of numbers, and any
//! value possibly bound to a user property as `{"user": "name", "value": ...}`.

use serde_json::Value;

/// The value itself, whether it is written plainly or bound to a user property.
pub fn plain(value: &Value) -> &Value {
    match value {
        Value::Object(fields) if fields.contains_key("value") => &fields["value"],
        other => other,
    }
}

pub fn number(value: Option<&Value>) -> Option<f32> {
    match plain(value?) {
        Value::Number(n) => n.as_f64().map(|n| n as f32),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(on) => Some(if *on { 1.0 } else { 0.0 }),
        _ => None,
    }
}

pub fn flag(value: Option<&Value>) -> Option<bool> {
    match plain(value?) {
        Value::Bool(on) => Some(*on),
        Value::Number(n) => n.as_f64().map(|n| n != 0.0),
        Value::String(text) => match text.trim() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

pub fn text(value: Option<&Value>) -> Option<&str> {
    plain(value?).as_str()
}

/// `"x y z"`, or an array of numbers, filled out with `fill` where it falls short.
pub fn vector<const N: usize>(value: Option<&Value>, fill: [f32; N]) -> [f32; N] {
    let numbers: Vec<f32> = match value.map(plain) {
        Some(Value::String(text)) => text
            .split_whitespace()
            .filter_map(|n| n.parse().ok())
            .collect(),
        Some(Value::Array(items)) => items.iter().filter_map(|n| number(Some(n))).collect(),
        Some(Value::Number(n)) => n.as_f64().map(|n| vec![n as f32]).unwrap_or_default(),
        _ => Vec::new(),
    };
    std::array::from_fn(|at| numbers.get(at).copied().unwrap_or(fill[at]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vectors_come_from_strings_arrays_and_user_bindings() {
        assert_eq!(
            vector(Some(&json!("960.0 540.5 0")), [0.0; 3]),
            [960.0, 540.5, 0.0]
        );
        assert_eq!(
            vector(Some(&json!([1, 2])), [0.0, 0.0, 9.0]),
            [1.0, 2.0, 9.0]
        );
        assert_eq!(
            vector(
                Some(&json!({"user": "tint", "value": "0.5 0.5 1"})),
                [1.0; 3]
            ),
            [0.5, 0.5, 1.0]
        );
        assert_eq!(vector::<2>(None, [1.0, 1.0]), [1.0, 1.0]);
    }

    #[test]
    fn numbers_and_flags_take_what_they_are_given() {
        assert_eq!(number(Some(&json!("0.25"))), Some(0.25));
        assert_eq!(
            number(Some(&json!({"user": "opacity", "value": 0.5}))),
            Some(0.5)
        );
        assert_eq!(
            flag(Some(&json!({"user": "show", "value": false}))),
            Some(false)
        );
        assert_eq!(flag(Some(&json!(1))), Some(true));
        assert_eq!(text(Some(&json!("models/a.json"))), Some("models/a.json"));
        assert_eq!(number(None), None);
    }
}
