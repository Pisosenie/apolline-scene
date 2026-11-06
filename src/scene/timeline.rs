//! Timelines: values animated by keyframes, as Wallpaper Engine's editor saves them. Each
//! component has its own channel of keys (`c0`, `c1`, …), and each key's handles say how the
//! curve leaves and reaches it. Between two keys the value follows a cubic with the slopes the
//! handles give: flat for the editor's default handles, which eases in and out, the slope
//! through the neighbouring keys for automatic (`magic`) ones, and a straight line for a
//! disabled handle. The editor's `previewvalue` beside them is left alone.

use serde_json::Value;

use crate::value::{flag, number, text};

#[derive(Clone, Debug, PartialEq)]
pub struct Timeline {
    /// What scripts find it by.
    pub name: String,
    pub fps: f32,
    /// Its length in frames.
    pub length: f32,
    pub mode: Mode,
    /// A loop runs from its last key back to its first, rather than holding the last.
    pub wrap: bool,
    /// It waits at its start until a script plays it.
    pub paused: bool,
    /// Its values are added to the value it's on, rather than standing for it.
    pub relative: bool,
    /// Each component's keys, in frame order; a component without keys keeps its value.
    pub channels: Vec<Vec<Key>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Loop,
    /// Once, then holding its end.
    Single,
    /// Forwards, then backwards, and again.
    Mirror,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub frame: f32,
    pub value: f32,
    /// How fast the value changes reaching the key and leaving it, per frame.
    pub slopes: [f32; 2],
}

const OPTIONS: &[&str] = &[
    "children",
    "events",
    "fps",
    "length",
    "mode",
    "name",
    "parent",
    "startpaused",
    "wraploop",
];
const KEYS: usize = 1024;

impl Timeline {
    /// A value's `animation`, or why it can't be played.
    pub fn parse(animation: &Value) -> Result<Self, String> {
        let fields = animation
            .as_object()
            .ok_or("an animation that isn't an object")?;
        let options = fields
            .get("options")
            .and_then(Value::as_object)
            .ok_or("an animation without options")?;
        if let Some(key) = options.keys().find(|key| !OPTIONS.contains(&key.as_str())) {
            return Err(format!("an animation option {}", short(key)));
        }
        if let Some(key) = fields.keys().find(|key| {
            !matches!(key.as_str(), "options" | "relative" | "previewvalue")
                && channel(key).is_none()
        }) {
            return Err(format!("an animation field {}", short(key)));
        }
        let fps = number(options.get("fps"))
            .filter(|fps| *fps > 0.0 && *fps <= 1000.0)
            .ok_or("an animation's frame rate")?;
        let length = number(options.get("length"))
            .filter(|length| *length >= 1.0 && *length <= 1e6)
            .ok_or("an animation's length")?;
        let mode = match text(options.get("mode")) {
            Some("loop") | None => Mode::Loop,
            Some("single") => Mode::Single,
            Some("mirror") => Mode::Mirror,
            Some(_) => return Err("an animation played in an unknown way".into()),
        };
        let wrap = flag(options.get("wraploop")).unwrap_or(false);
        let wraps = (wrap && mode == Mode::Loop).then_some(length);
        let mut channels = Vec::new();
        for (key, keys) in fields {
            let Some(at) = channel(key) else {
                continue;
            };
            let keys = keys
                .as_array()
                .ok_or("an animation channel that isn't a list")?;
            if keys.is_empty() || keys.len() > KEYS {
                return Err("an animation channel without keys or with too many".into());
            }
            if channels.len() <= at {
                channels.resize(at + 1, Vec::new());
            }
            channels[at] = slopes(keys, wraps)?;
        }
        Ok(Timeline {
            name: text(options.get("name"))
                .unwrap_or_default()
                .chars()
                .take(256)
                .collect(),
            fps,
            length,
            mode,
            wrap,
            paused: flag(options.get("startpaused")).unwrap_or(false),
            relative: flag(fields.get("relative")).unwrap_or(false),
            channels,
        })
    }

    /// For scripts to play: its options and each channel's keys as
    /// `[frame, value, slope in, slope out]`.
    pub(crate) fn to_json(&self) -> Value {
        let mode = match self.mode {
            Mode::Loop => "loop",
            Mode::Single => "single",
            Mode::Mirror => "mirror",
        };
        let channels: Vec<Vec<[f32; 4]>> = self
            .channels
            .iter()
            .map(|keys| {
                keys.iter()
                    .map(|key| [key.frame, key.value, key.slopes[0], key.slopes[1]])
                    .collect()
            })
            .collect();
        serde_json::json!({
            "name": self.name,
            "fps": self.fps,
            "length": self.length,
            "mode": mode,
            "wrap": self.wrap,
            "paused": self.paused,
            "relative": self.relative,
            "channels": channels,
        })
    }
}

/// `c0` → 0, up to four components.
pub(crate) fn channel(key: &str) -> Option<usize> {
    ["c0", "c1", "c2", "c3"]
        .iter()
        .position(|name| *name == key)
}

/// A channel's keys in frame order, with the slopes their handles give. In a loop `wraps`
/// long, the first key follows the last.
fn slopes(keys: &[Value], wraps: Option<f32>) -> Result<Vec<Key>, String> {
    let mut read: Vec<(f32, f32, [&Value; 2])> = Vec::with_capacity(keys.len());
    for key in keys {
        let frame = number(key.get("frame")).filter(|frame| frame.abs() <= 1e6);
        let value = number(key.get("value")).filter(|value| value.abs() <= 1e9);
        let (Some(frame), Some(value)) = (frame, value) else {
            return Err("an animation key without a frame or value".into());
        };
        let handle = |side: &str| key.get(side).unwrap_or(&Value::Null);
        read.push((frame, value, [handle("back"), handle("front")]));
    }
    read.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::with_capacity(read.len());
    for at in 0..read.len() {
        let (frame, value, handles) = read[at];
        let last = read.len() - 1;
        // Round a loop, the nearest key a loop's length away on that side, past any that
        // lands on this one's frame.
        let before = match (at.checked_sub(1), wraps) {
            (Some(before), _) => Some(read[before]),
            (None, Some(length)) if last > 0 => read
                .iter()
                .rev()
                .map(|key| (key.0 - length, key.1, key.2))
                .find(|key| key.0 < frame),
            _ => None,
        };
        let after = match (read.get(at + 1), wraps) {
            (Some(after), _) => Some(*after),
            (None, Some(length)) if last > 0 => read
                .iter()
                .map(|key| (key.0 + length, key.1, key.2))
                .find(|key| key.0 > frame),
            _ => None,
        };
        let chord = |other: Option<(f32, f32, [&Value; 2])>| match other {
            Some((there, then, _)) if there != frame => (then - value) / (there - frame),
            _ => 0.0,
        };
        // The slope through both neighbours, flat at the ends.
        let through = match (before, after) {
            (Some((first, from, _)), Some((last, to, _))) if last > first => {
                (to - from) / (last - first)
            }
            _ => 0.0,
        };
        let side = |handle: &Value, neighbour: Option<(f32, f32, [&Value; 2])>| {
            if flag(handle.get("enabled")) == Some(false) {
                return chord(neighbour);
            }
            if flag(handle.get("magic")) == Some(true) {
                return through;
            }
            // A handle dragged by hand: its rise over its run, in the editor's units, which
            // are a fifth of a frame across the span to the neighbour.
            let x = number(handle.get("x")).unwrap_or(0.0);
            let y = number(handle.get("y")).unwrap_or(0.0);
            let span = neighbour.map_or(0.0, |(there, ..)| (there - frame).abs());
            if x.abs() < 1e-6 || span < 1e-6 {
                0.0
            } else {
                (5.0 * y / (x * span)).clamp(-1e6, 1e6)
            }
        };
        out.push(Key {
            frame,
            value,
            slopes: [side(handles[0], before), side(handles[1], after)],
        });
    }
    Ok(out)
}

fn short(key: &str) -> String {
    key.chars().take(64).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(frame: f32, value: f32, magic: bool) -> Value {
        json!({"frame": frame, "value": value,
               "back": {"enabled": true, "magic": magic, "x": -1, "y": 0},
               "front": {"enabled": true, "magic": magic, "x": 1, "y": 0},
               "lockangle": true, "locklength": true})
    }

    #[test]
    fn keys_come_in_frame_order_with_the_slopes_their_handles_give() {
        let timeline = Timeline::parse(&json!({
            "c0": [key(20.0, 0.0, true), key(0.0, 0.0, true), key(10.0, 5.0, true)],
            "c2": [key(0.0, 1.0, false), {"frame": 30, "value": 4,
                   "back": {"enabled": false, "x": -1, "y": 0}, "front": {"enabled": true, "x": 1, "y": 0}}],
            "options": {"fps": 30, "length": 60, "mode": "mirror", "wraploop": null, "startpaused": true, "name": "wave"},
            "relative": true
        }))
        .unwrap();
        assert_eq!(
            (timeline.mode, timeline.paused, timeline.relative),
            (Mode::Mirror, true, true)
        );
        assert_eq!(timeline.name, "wave");
        let frames: Vec<f32> = timeline.channels[0].iter().map(|key| key.frame).collect();
        assert_eq!(frames, [0.0, 10.0, 20.0]);
        // Automatic handles take the slope through the neighbours, flat at the ends.
        assert_eq!(timeline.channels[0][1].slopes, [0.0, 0.0]);
        assert_eq!(timeline.channels[0][0].slopes, [0.0, 0.0]);
        assert!(timeline.channels[1].is_empty());
        // A default handle is flat; a disabled one runs straight at its neighbour.
        assert_eq!(timeline.channels[2][0].slopes, [0.0, 0.0]);
        assert_eq!(timeline.channels[2][1].slopes[0], 0.1);
    }

    #[test]
    fn hand_dragged_handles_rise_the_way_they_point_and_loops_join_smoothly() {
        let dragged = |frame: f32, value: f32| {
            json!({"frame": frame, "value": value,
                   "back": {"enabled": true, "x": -1, "y": -1}, "front": {"enabled": true, "x": 1, "y": 1}})
        };
        let rising = Timeline::parse(&json!({
            "c0": [dragged(0.0, 0.0), dragged(10.0, 5.0), dragged(20.0, 10.0)],
            "options": {"fps": 30, "length": 20}
        }))
        .unwrap();
        let slopes = rising.channels[0][1].slopes;
        assert!(slopes[0] > 0.0 && slopes[0] == slopes[1], "{slopes:?}");
        // Keys at both ends of a wrapping loop: each end looks past the other.
        let looped = Timeline::parse(&json!({
            "c0": [key(0.0, 0.0, true), key(10.0, 10.0, true), key(20.0, 10.0, true),
                   key(30.0, 0.0, true), key(40.0, 0.0, true)],
            "options": {"fps": 30, "length": 40, "mode": "loop", "wraploop": true}
        }))
        .unwrap();
        let keys = &looped.channels[0];
        assert_eq!(keys[0].slopes, [0.5, 0.5]);
        assert_eq!(keys[4].slopes, [0.5, 0.5]);
    }

    #[test]
    fn what_cant_be_played_is_named() {
        for (animation, why) in [
            (
                json!({"c0": [key(0.0, 0.0, false)], "options": {"fps": 0, "length": 10}}),
                "frame rate",
            ),
            (
                json!({"c0": [key(0.0, 0.0, false)], "options": {"fps": 30, "length": 10, "mode": "bounce"}}),
                "unknown way",
            ),
            (
                json!({"c0": [], "options": {"fps": 30, "length": 10}}),
                "without keys",
            ),
            (
                json!({"c9": [key(0.0, 0.0, false)], "options": {"fps": 30, "length": 10}}),
                "field c9",
            ),
            (
                json!({"c00": [key(0.0, 0.0, false)], "options": {"fps": 30, "length": 10}}),
                "field c00",
            ),
            (
                json!({"c0": [key(0.0, 0.0, false)], "options": {"fps": 30, "length": 10, "speed": 2}}),
                "option speed",
            ),
            (
                json!({"c0": [{"frame": 0}], "options": {"fps": 30, "length": 10}}),
                "without a frame or value",
            ),
        ] {
            let error = Timeline::parse(&animation).unwrap_err();
            assert!(error.contains(why), "{why}: {error}");
        }
    }
}
