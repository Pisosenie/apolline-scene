//! Control points: up to eight places a system's operators pull its particles towards or
//! push them from, each its offset from the system's origin or, locked to the pointer,
//! wherever the pointer is.

use serde_json::Value;

use crate::Error;
use crate::value::{flag, number, vector};

/// How many control points a system has.
pub const CONTROLS: usize = 8;
/// The flag that locks a control point to the pointer.
const POINTER: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Control {
    pub offset: [f32; 3],
    /// It follows the pointer, offset from it.
    pub pointer: bool,
    /// Other flags, which nothing here understands; an operator that uses it is refused.
    pub unknown: bool,
}

/// The system's control points by id; those it doesn't list sit at its origin.
pub(super) fn controls(root: &Value) -> [Control; CONTROLS] {
    let mut controls = [Control::default(); CONTROLS];
    let listed = root.get("controlpoint").and_then(Value::as_array);
    for point in listed.into_iter().flatten() {
        let Some(id) = number(point.get("id"))
            .filter(|id| id.fract() == 0.0 && (0.0..CONTROLS as f32).contains(id))
            .map(|id| id as usize)
        else {
            continue;
        };
        let flags = number(point.get("flags")).unwrap_or(0.0) as u32;
        controls[id] = Control {
            offset: vector(point.get("offset"), [0.0; 3]),
            pointer: flags & POINTER != 0 || flag(point.get("locktopointer")) == Some(true),
            unknown: flags & !POINTER != 0,
        };
    }
    controls
}

/// Which control point an operator names, refused when it's one nothing here can place.
pub(super) fn named(
    value: &Value,
    controls: &[Control; CONTROLS],
    refuse: impl Fn(&str) -> Error,
) -> Result<usize, Error> {
    let at = number(value.get("controlpoint")).unwrap_or(0.0);
    if at.fract() != 0.0 || !(0.0..CONTROLS as f32).contains(&at) {
        return Err(refuse("a control point that isn't 0 to 7"));
    }
    if controls[at as usize].unknown {
        return Err(refuse("control point flags"));
    }
    Ok(at as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn control_points_sit_at_their_offsets_or_follow_the_pointer() {
        let root = json!({"controlpoint": [
            {"id": 1, "flags": 1, "offset": "0 0 0"},
            {"id": 2, "flags": 0, "offset": "100 0 0"},
            {"id": 3, "locktopointer": true},
            {"id": 4, "flags": 16},
            {"id": 9, "flags": 1}
        ]});
        let controls = controls(&root);
        assert!(controls[1].pointer && controls[3].pointer && !controls[2].pointer);
        assert_eq!(controls[2].offset, [100.0, 0.0, 0.0]);
        let refuse = |what: &str| Error::Format(what.into());
        assert_eq!(
            named(&json!({"controlpoint": 2}), &controls, refuse).unwrap(),
            2
        );
        assert_eq!(named(&json!({}), &controls, refuse).unwrap(), 0);
        assert!(named(&json!({"controlpoint": 4}), &controls, refuse).is_err());
        assert!(named(&json!({"controlpoint": 8}), &controls, refuse).is_err());
    }
}
