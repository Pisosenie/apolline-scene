//! Text layers' words drawn anew when scripts change them, into the same texture and the same
//! effect targets, at the new words' size.

use std::sync::Arc;

use super::{Base, Drawn, reserve, upload::fill};
use crate::text::Lettering;

pub(super) struct Words {
    pub lettering: Arc<Lettering>,
    /// The words drawn last; none while they're still the ones the layer was prepared with.
    pub shown: Option<String>,
    /// The texture they're drawn into, which the layer's own step reads.
    pub texture: glow::Texture,
    /// What it and the layer's effect targets take, counted in the renderer's budget.
    pub bytes: usize,
}

/// What words of this size take: their texture with its mipmaps, and two effect targets.
pub(super) fn bytes(texture: &crate::tex::Texture, chained: bool) -> usize {
    let mips: usize = texture.images[0].iter().map(|mip| mip.pixels.len()).sum();
    let targets = 2 * 4 * texture.width as usize * texture.height as usize;
    mips + if chained { targets } else { 0 }
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
        let bytes = bytes(&raster.texture, self.chain.is_some());
        let mut total = allocated.saturating_sub(words.bytes);
        if reserve(&mut total, bytes).is_err()
            || unsafe { fill(gl, words.texture, &raster.texture) }.is_err()
        {
            return;
        }
        *allocated = total;
        words.bytes = bytes;
        let (width, height) = (raster.texture.width, raster.texture.height);
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
