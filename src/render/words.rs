//! Text layers' words drawn anew when scripts change them, into the same texture and the same
//! effect targets, at the new words' size.

use std::sync::Arc;

use super::{Base, Drawn, reserve, upload::fill};
use crate::tex::Texture;
use crate::text::Lettering;

pub(super) struct Words {
    pub lettering: Arc<Lettering>,
    /// The words tried last, which aren't tried again until they change.
    pub shown: Option<String>,
    /// The words in the texture now; none while they're the ones the layer was prepared with.
    pub drawn: Option<String>,
    /// The picture of the words the layer was prepared with.
    pub prepared: Arc<Texture>,
    /// The texture they're drawn into, which the layer's own step reads.
    pub texture: glow::Texture,
    /// What the texture takes, counted in the renderer's budget.
    pub bytes: usize,
}

/// What a picture of words takes: its texture with its mipmaps.
pub(super) fn bytes(texture: &Texture) -> usize {
    texture.images[0].iter().map(|mip| mip.pixels.len()).sum()
}

impl Drawn {
    /// Draws new words in place of the old and places the layer by their size, within the
    /// renderer's budget, which `allocated` is all of its layers' share of.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    pub(super) unsafe fn retext(&mut self, gl: &glow::Context, text: &str, allocated: &mut usize) {
        let Some(words) = self.words.as_mut() else {
            return;
        };
        if words.shown.as_deref() == Some(text) {
            return;
        }
        // Words that don't fit or that GL won't take leave the layer as it was, and aren't
        // tried again until they change.
        words.shown = Some(text.to_owned());
        let raster = words.lettering.raster(text);
        let (width, height) = (raster.texture.width, raster.texture.height);
        let (was, will) = self.chain.as_ref().map_or((0, 0), |chain| {
            (chain.bytes(chain.size()), chain.bytes((width, height)))
        });
        let bytes = bytes(&raster.texture);
        let mut total = allocated.saturating_sub(words.bytes + was);
        if reserve(&mut total, bytes + will).is_err() {
            return;
        }
        if unsafe { fill(gl, words.texture, &raster.texture) }.is_err() {
            // GL may have taken some of its levels; the old words go back in whole.
            let old = match &words.drawn {
                Some(drawn) => &words.lettering.raster(drawn).texture,
                None => &*words.prepared,
            };
            let _ = unsafe { fill(gl, words.texture, old) };
            return;
        }
        *allocated = total;
        words.bytes = bytes;
        words.drawn = Some(text.to_owned());
        let whole = [width as f32, height as f32, width as f32, height as f32];
        let own = match self.chain.as_mut() {
            Some(chain) => {
                unsafe { chain.resize(gl, (width, height)) };
                self.step.inputs[0].resolution = whole;
                chain.own()
            }
            None => &mut self.step,
        };
        own.inputs[0].resolution = whole;
        own.uv_max = [1.0; 2];
        if let Base::Picture { size, anchor, .. } = &mut self.base {
            *size = raster.size;
            *anchor = raster.anchor;
        }
        let pose = self.posed;
        self.pose(&pose);
    }
}
