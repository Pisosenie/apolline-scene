//! Sprite sheets: a layer whose texture's frames play in turn, each shown for its own time and
//! the whole looping, by moving where the layer's picture is read from in the sheet.

use super::Drawn;

pub(super) struct Sprite {
    /// Each frame's image, where it is in it as `[corner, across, down]`, and when in the loop
    /// it ends, in seconds, added up finely enough that the loop's end is where it says.
    frames: Vec<(glow::Texture, [f32; 6], f64)>,
    /// The frame drawn last.
    shown: Option<usize>,
}

impl Sprite {
    /// `images` are the texture's images on the GPU, in order.
    pub fn new(frames: &[crate::tex::Frame], images: &[glow::Texture]) -> Self {
        let mut end = 0.0;
        let frames = frames
            .iter()
            .map(|frame| {
                // Times are written to a few decimals; as floats a tenth is a little over.
                end += (f64::from(frame.seconds) * 1e6).round() / 1e6;
                let place = [
                    frame.corner[0],
                    frame.corner[1],
                    frame.across[0],
                    frame.across[1],
                    frame.down[0],
                    frame.down[1],
                ];
                (images[frame.image], place, end)
            })
            .collect();
        Sprite {
            frames,
            shown: None,
        }
    }

    /// The frame to show `time` seconds in.
    fn at(&self, time: f32) -> usize {
        let total = self.frames.last().map_or(0.0, |frame| frame.2);
        if total <= 0.0 || !time.is_finite() {
            return 0;
        }
        let into = f64::from(time).rem_euclid(total);
        self.frames
            .partition_point(|frame| frame.2 <= into)
            .min(self.frames.len() - 1)
    }
}

impl Drawn {
    /// Shows the sprite sheet's frame for `time`, drawing the layer's effects again when it
    /// changes.
    pub(super) fn play(&mut self, time: f32) {
        let Some(sprite) = self.sprite.as_mut() else {
            return;
        };
        let at = sprite.at(time);
        if sprite.shown == Some(at) {
            return;
        }
        sprite.shown = Some(at);
        let (texture, place, _) = sprite.frames[at];
        let own = match self.chain.as_mut() {
            Some(chain) => {
                chain.touch();
                chain.own()
            }
            None => &mut self.step,
        };
        own.inputs[0].texture = texture;
        own.sheet = Some(place);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tex::Frame;

    fn frame(image: usize, seconds: f32, x: f32) -> Frame {
        Frame {
            image,
            seconds,
            corner: [x, 0.0],
            across: [0.25, 0.0],
            down: [0.0, 1.0],
            size: (1.0, 1.0),
        }
    }

    #[test]
    fn frames_show_for_their_own_time_and_loop() {
        let texture = glow::NativeTexture(std::num::NonZeroU32::new(7).unwrap());
        let sprite = Sprite::new(
            &[frame(0, 0.1, 0.0), frame(0, 0.3, 0.25), frame(0, 0.1, 0.5)],
            &[texture],
        );
        let shown: Vec<usize> = [0.0, 0.05, 0.1, 0.39, 0.4, 0.45, 0.5, 0.55, -0.05]
            .iter()
            .map(|time| sprite.at(*time))
            .collect();
        assert_eq!(shown, [0, 0, 1, 1, 2, 2, 0, 0, 2]);
        let still = Sprite::new(&[frame(0, 0.0, 0.0), frame(0, 0.0, 0.5)], &[texture]);
        assert_eq!(still.at(3.0), 0);
        // Ten tenths make a second: a second in, the loop is back at its start.
        let tenths = Sprite::new(&vec![frame(0, 0.1, 0.0); 10], &[texture]);
        assert_eq!((tenths.at(1.0), tenths.at(0.95)), (0, 9));
    }
}
