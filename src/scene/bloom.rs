//! The scene's bloom, from its general settings: how strongly its bright parts glow, from how
//! bright, and in what colour. HDR bloom has settings of its own and isn't read here.

use serde_json::Value;

use crate::value::{flag, number, vector};

/// A glow around the parts of the scene brighter than `threshold`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bloom {
    pub strength: f32,
    /// How bright a pixel's brightest channel has to be to glow, 0 to 1.
    pub threshold: f32,
    pub tint: [f32; 3],
}

/// The bloom the general settings turn on, with Wallpaper Engine's defaults for the rest.
pub(super) fn read<'a>(field: impl Fn(&str) -> Option<&'a Value>) -> Option<Bloom> {
    flag(field("bloom")).unwrap_or(false).then(|| Bloom {
        strength: number(field("bloomstrength"))
            .unwrap_or(2.0)
            .clamp(0.0, 16.0),
        threshold: number(field("bloomthreshold"))
            .unwrap_or(0.65)
            .clamp(0.0, 1.0),
        tint: vector(field("bloomtint"), [1.0; 3]).map(|channel| channel.clamp(0.0, 16.0)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bloom_is_read_when_turned_on_with_defaults_for_the_rest() {
        let general = json!({"bloom": true, "bloomstrength": 0.5, "bloomtint": "1 0.5 0"});
        let bloom = read(|name| general.get(name)).unwrap();
        assert_eq!(
            bloom,
            Bloom {
                strength: 0.5,
                threshold: 0.65,
                tint: [1.0, 0.5, 0.0]
            }
        );
        let off = json!({"bloom": false, "bloomstrength": 3});
        assert_eq!(read(|name| off.get(name)), None);
    }
}
