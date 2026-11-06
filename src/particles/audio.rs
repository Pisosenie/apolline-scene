//! Audio response: an emitter that lets particles out as loud as the music is, over a range
//! of its frequencies. Quieter than the lower bound it lets none out, louder than the upper
//! all it asks for, and in between a share bent by the exponent.

use serde_json::Value;

use crate::Error;
use crate::value::{number, vector};

/// The keys of an emitter's audio response.
pub(super) const KEYS: &[&str] = &[
    "audioprocessingbounds",
    "audioprocessingexponent",
    "audioprocessingfrequencyend",
    "audioprocessingfrequencystart",
    "audioprocessingmode",
];
/// Both channels together; the editor's other modes, one channel each, aren't read.
const BOTH: f32 = 3.0;
/// The editor's frequency bands, each four of the spectrum's.
const BANDS: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hearing {
    /// The first and last band heard, of 16.
    pub bands: [usize; 2],
    pub bounds: [f32; 2],
    pub exponent: f32,
}

impl Hearing {
    /// What share of its rate the emitter lets out with this left and right spectrum.
    pub fn share(&self, spectrum: &[[f32; 64]; 2]) -> f32 {
        let bins = self.bands[0] * 4..(self.bands[1] + 1) * 4;
        let count = (bins.len() * 2).max(1) as f32;
        let loudness = spectrum
            .iter()
            .flat_map(|channel| &channel[bins.clone()])
            .sum::<f32>()
            / count;
        let [low, high] = self.bounds;
        let along = if high - low > 1e-6 {
            ((loudness - low) / (high - low)).clamp(0.0, 1.0)
        } else if loudness >= low {
            1.0
        } else {
            0.0
        };
        along.powf(self.exponent)
    }
}

/// An emitter's audio response, if it has one, refused when it listens to one channel.
pub(super) fn hearing(
    value: &Value,
    refuse: impl Fn(&str) -> Error,
) -> Result<Option<Hearing>, Error> {
    let mode = number(value.get("audioprocessingmode")).unwrap_or(0.0);
    if mode == 0.0 {
        return Ok(None);
    }
    if mode != BOTH {
        return Err(refuse("audio response on one channel"));
    }
    let band = |key: &str, fill: usize| {
        number(value.get(key)).map_or(fill, |band| (band.max(0.0) as usize).min(BANDS - 1))
    };
    let first = band("audioprocessingfrequencystart", 0);
    let bounds = vector(value.get("audioprocessingbounds"), [0.0, 1.0]);
    Ok(Some(Hearing {
        bands: [
            first,
            band("audioprocessingfrequencyend", BANDS - 1).max(first),
        ],
        bounds: [bounds[0].min(bounds[1]), bounds[1].max(bounds[0])],
        exponent: number(value.get("audioprocessingexponent"))
            .unwrap_or(1.0)
            .clamp(0.01, 100.0),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_emitter_hears_its_bands_between_its_bounds() {
        let refuse = |what: &str| Error::Format(what.into());
        let value = json!({"audioprocessingmode": 3, "audioprocessingbounds": "0.5 1",
                           "audioprocessingfrequencyend": 1});
        let bass = hearing(&value, refuse).unwrap().unwrap();
        assert_eq!(bass.bands, [0, 1]);
        let mut spectrum = [[0.0; 64]; 2];
        assert_eq!(bass.share(&spectrum), 0.0);
        // The bass at three quarters is halfway up from the lower bound.
        for channel in &mut spectrum {
            channel[..8].fill(0.75);
        }
        assert_eq!(bass.share(&spectrum), 0.5);
        // Loud treble outside its bands changes nothing.
        spectrum[0][32..].fill(1.0);
        assert_eq!(bass.share(&spectrum), 0.5);
        assert_eq!(hearing(&json!({}), refuse).unwrap(), None);
        assert!(hearing(&json!({"audioprocessingmode": 1}), refuse).is_err());
    }
}
