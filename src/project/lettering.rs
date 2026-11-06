//! Text layers: their fonts read once per preparation, their words drawn as they first stand,
//! and the lettering kept for the renderer to draw new words when scripts change them.

use std::collections::HashMap;
use std::sync::Arc;

use ab_glyph::FontArc;

use super::{Blending, Layer, Project, diagnostic, material::Resources};
use crate::Error;
use crate::scene::{Image, Text};
use crate::text::{self, Lettering};

/// Wallpaper Engine's own font, for what a layer's font has no glyphs for.
const STOCK: &str = "fonts/NotoSans-Regular.ttf";

pub(super) type Fonts = HashMap<String, Result<FontArc, String>>;

impl Project {
    pub(super) fn text_layer(
        &self,
        words: &Text,
        fonts: &mut Fonts,
        resources: &mut Resources,
        remaining: &mut usize,
        still: bool,
    ) -> Result<Layer, Error> {
        let font = self.font(&words.font, fonts, remaining)?;
        let others: Vec<FontArc> = (words.font != STOCK)
            .then(|| self.font(STOCK, fonts, remaining).ok())
            .flatten()
            .into_iter()
            .collect();
        let (effects, undrawn) = self.effects(&words.layer, resources, remaining, still)?;
        let lettering = Lettering::new(font, others, words, !effects.is_empty());
        let raster = lettering.raster(&words.text);
        let bytes: usize = raster.texture.images[0]
            .iter()
            .map(|mip| mip.pixels.len())
            .sum();
        *remaining = remaining
            .checked_sub(bytes)
            .ok_or_else(|| Error::Format("scene textures exceed their budget".into()))?;
        Ok(Layer {
            object: 0,
            image: Image {
                size: Some(raster.size),
                anchor: raster.anchor,
                ..words.layer.clone()
            },
            shade: [1.0; 2],
            power: 1.0,
            texture: Arc::new(raster.texture),
            secondary: Vec::new(),
            fragment: None,
            blending: Blending::Translucent,
            fullscreen: false,
            effects,
            undrawn,
            sprite: false,
            compose: false,
            particles: None,
            child: None,
            leader: None,
            puppet: None,
            text: Some(Arc::new(lettering)),
        })
    }

    /// A font by the name a text layer gives, read once however many layers use it, its bytes
    /// counted against the scene's budget.
    fn font(&self, name: &str, fonts: &mut Fonts, remaining: &mut usize) -> Result<FontArc, Error> {
        fonts
            .entry(name.to_owned())
            .or_insert_with(|| {
                let (font, size) = self.read_font(name).map_err(diagnostic)?;
                *remaining = remaining
                    .checked_sub(size)
                    .ok_or_else(|| format!("{name}: the scene's fonts exceed its budget"))?;
                Ok(font)
            })
            .clone()
            .map_err(Error::Format)
    }

    fn read_font(&self, name: &str) -> Result<(FontArc, usize), Error> {
        if let Some(system) = name.strip_prefix("systemfont_") {
            return text::system(system, |stock| Some(self.file(stock)?.into_owned()));
        }
        let bytes = self
            .try_file(name, text::FONT_BYTES)?
            .ok_or_else(|| Error::Format(format!("{name} isn't in the project")))?;
        let size = bytes.len();
        Ok((text::parse(bytes.into_owned())?, size))
    }
}
