//! Text layers' words drawn into textures, sized the way Wallpaper Engine sizes them: a point
//! is 1/300 of an inch, 25/6 scene pixels; a line is the font's ascent, descent and gap; the
//! words' box is as wide as their widest line and as tall as all their lines. Padding is clear
//! room around the box for effects to spread into. The texture holds white words in their
//! coverage, for the layer's colour to tint.

mod fonts;

pub(crate) use fonts::{FONT_BYTES, parse, system, warm};

use ab_glyph::{Font, FontArc, GlyphId, PxScale, point};

use crate::scene::Text;
use crate::tex::{CLAMP_UVS, Channels, Format, Mip, Texture};

/// Scene pixels to a point.
const POINT: f32 = 300.0 / 72.0;
/// The most texels a text texture has on a side, and in all.
const SIDE: f32 = 4096.0;
const AREA: f32 = 4.0 * 1024.0 * 1024.0;
/// The most characters of a text that are drawn.
pub const CHARACTERS: usize = 4096;
/// How much finer than scene pixels words are drawn, per unit of their scale, when no effect
/// works on their texels.
const SHARPER: f32 = 2.0;

/// How a text layer's words are drawn, whatever they say.
pub struct Lettering {
    /// The layer's font first, then fonts for what it has no glyphs for.
    fonts: Vec<FontArc>,
    /// Scene pixels to an em.
    em: f32,
    align: [f32; 2],
    padding: [f32; 2],
    /// The widest a line gets, in scene pixels, and the most lines.
    width: Option<f32>,
    rows: Option<usize>,
    ellipsis: bool,
    /// Texels to a scene pixel.
    texel: f32,
}

/// Words drawn: their texture, the quad it covers in scene pixels, and where the layer's
/// origin is on the quad, from its centre in shares of its size, y up.
pub struct Raster {
    pub texture: Texture,
    pub size: (f32, f32),
    pub anchor: [f32; 2],
}

#[derive(Clone, Copy)]
struct Letter {
    character: char,
    font: usize,
    id: GlyphId,
    /// In ems.
    advance: f32,
}

impl Lettering {
    /// `font` is the layer's own; `others` stand in for what it has no glyphs for.
    pub fn new(font: FontArc, others: Vec<FontArc>, text: &Text, effects: bool) -> Self {
        let scale = text.layer.scale[0].abs().max(text.layer.scale[1].abs());
        let mut fonts = vec![font];
        fonts.extend(others);
        Lettering {
            fonts,
            em: text.points.clamp(0.0, 2000.0) * POINT,
            align: text.align,
            padding: text.padding.map(|pad| pad.clamp(0.0, SIDE)),
            width: text
                .width
                .filter(|width| *width > 0.0)
                .map(|width| width.min(1e6)),
            rows: text.rows.filter(|rows| *rows > 0),
            ellipsis: text.ellipsis,
            texel: if effects {
                1.0
            } else {
                (scale * SHARPER).clamp(1.0, 4.0)
            },
        }
    }

    /// The words drawn, laid out, wrapped and cut to the layer's limits.
    pub fn raster(&self, text: &str) -> Raster {
        let lines = self.lines(text);
        let placed: Vec<(Vec<f32>, f32)> = lines.iter().map(|line| self.places(line)).collect();
        let (ascent, line) = self.metrics();
        let widest = placed.iter().map(|(_, width)| *width).fold(0.0, f32::max);
        let quad = [
            (widest * self.em + 2.0 * self.padding[0]).max(1.0),
            (lines.len() as f32 * line * self.em + 2.0 * self.padding[1]).max(1.0),
        ];
        let texel = self
            .texel
            .min(SIDE / quad[0])
            .min(SIDE / quad[1])
            .min((AREA / (quad[0] * quad[1])).sqrt());
        let [width, height] = quad.map(|side| ((side * texel).ceil() as u32).clamp(1, SIDE as u32));
        let mut coverage = vec![0u8; width as usize * height as usize];
        let em = self.em * texel;
        for (row, (letters, (places, used))) in lines.iter().zip(&placed).enumerate() {
            let start =
                self.padding[0] * texel + (widest - used) * (self.align[0] + 1.0) / 2.0 * em;
            let baseline = self.padding[1] * texel + (row as f32 * line + ascent) * em;
            for (letter, place) in letters.iter().zip(places) {
                let font = self.font(letter.font);
                let scale = PxScale::from(em * font.height_unscaled() / units(font));
                let glyph = letter
                    .id
                    .with_scale_and_position(scale, point(start + place * em, baseline));
                let Some(outline) = font.outline_glyph(glyph) else {
                    continue;
                };
                let corner = outline.px_bounds().min;
                outline.draw(|x, y, cover| {
                    let x = corner.x as i64 + i64::from(x);
                    let y = corner.y as i64 + i64::from(y);
                    if (0..i64::from(width)).contains(&x) && (0..i64::from(height)).contains(&y) {
                        let at = &mut coverage[y as usize * width as usize + x as usize];
                        *at = at.saturating_add((cover.clamp(0.0, 1.0) * 255.0).round() as u8);
                    }
                });
            }
        }
        let size = (width as f32 / texel, height as f32 / texel);
        Raster {
            texture: texture(width, height, coverage),
            size,
            anchor: [
                self.align[0] * (0.5 - self.padding[0] / size.0),
                self.align[1] * (0.5 - self.padding[1] / size.1),
            ],
        }
    }

    /// The layer's font's ascent and line height, in ems.
    fn metrics(&self) -> (f32, f32) {
        let font = &self.fonts[0];
        let units = units(font);
        let line = font.ascent_unscaled() - font.descent_unscaled() + font.line_gap_unscaled();
        (font.ascent_unscaled() / units, line.max(0.0) / units)
    }

    fn font(&self, at: usize) -> &FontArc {
        match self.fonts.get(at) {
            Some(font) => font,
            None => fonts::fallback().unwrap_or(&self.fonts[0]),
        }
    }

    /// The first of the layer's fonts with a glyph for it, then the machine's wide font.
    fn find(&self, character: char) -> Option<Letter> {
        let letter = |font: usize, id: GlyphId| {
            let face = self.font(font);
            Letter {
                character,
                font,
                id,
                advance: face.h_advance_unscaled(id) / units(face),
            }
        };
        let found = self.fonts.iter().enumerate().find_map(|(at, font)| {
            let id = font.glyph_id(character);
            (id.0 != 0).then_some((at, id))
        });
        found
            .or_else(|| {
                let id = fonts::fallback()?.glyph_id(character);
                (id.0 != 0).then_some((self.fonts.len(), id))
            })
            .map(|(font, id)| letter(font, id))
    }

    /// Its glyph, or the layer's font's sign for a missing one.
    fn letter(&self, character: char) -> Letter {
        self.find(character).unwrap_or_else(|| Letter {
            character,
            font: 0,
            id: GlyphId(0),
            advance: self.fonts[0].h_advance_unscaled(GlyphId(0)) / units(&self.fonts[0]),
        })
    }

    fn lines(&self, text: &str) -> Vec<Vec<Letter>> {
        let text: String = text.chars().take(CHARACTERS).collect();
        let mut lines = Vec::new();
        if text.is_empty() {
            return lines;
        }
        for written in text.split('\n') {
            let letters = written
                .chars()
                .filter(|character| !character.is_control())
                .map(|character| self.letter(character))
                .collect();
            match self.width {
                Some(width) => wrap(letters, width / self.em.max(f32::MIN_POSITIVE), &mut lines),
                None => lines.push(letters),
            }
        }
        if let Some(rows) = self.rows
            && lines.len() > rows
        {
            lines.truncate(rows);
            if self.ellipsis
                && let Some(last) = lines.last_mut()
            {
                self.ellipsize(last);
            }
        }
        lines
    }

    /// Ends a cut line in an ellipsis, taking letters off it until the ellipsis fits.
    fn ellipsize(&self, line: &mut Vec<Letter>) {
        let dots = match self.find('…') {
            Some(dots) => vec![dots],
            None => vec![self.letter('.'); 3],
        };
        let room = self.width.map_or(f32::INFINITY, |width| width / self.em);
        let length: f32 = dots.iter().map(|dot| dot.advance).sum();
        while !line.is_empty() && advance(line) + length > room {
            line.pop();
        }
        trim(line);
        line.extend(dots);
    }

    /// Each letter's place along its line and the line's width, in ems, kerned.
    fn places(&self, line: &[Letter]) -> (Vec<f32>, f32) {
        let mut places = Vec::with_capacity(line.len());
        let mut x = 0.0;
        let mut before: Option<&Letter> = None;
        for letter in line {
            if let Some(before) = before.filter(|before| before.font == letter.font) {
                let font = self.font(letter.font);
                x += font.kern_unscaled(before.id, letter.id) / units(font);
            }
            places.push(x);
            x += letter.advance;
            before = Some(letter);
        }
        (places, x.max(0.0))
    }
}

fn units(font: &FontArc) -> f32 {
    font.units_per_em()
        .filter(|units| *units > 0.0)
        .unwrap_or(1000.0)
}

fn advance(line: &[Letter]) -> f32 {
    line.iter().map(|letter| letter.advance).sum()
}

fn trim(line: &mut Vec<Letter>) {
    while line.last().is_some_and(|letter| letter.character == ' ') {
        line.pop();
    }
}

/// Characters as wide as they are tall, which lines may break around, as in Chinese, Japanese
/// and Korean.
fn wide(character: char) -> bool {
    matches!(u32::from(character),
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

/// Breaks a line wherever it would be wider than `width` ems: after a space, around a wide
/// character, or inside a word too long for a line of its own.
fn wrap(letters: Vec<Letter>, width: f32, lines: &mut Vec<Vec<Letter>>) {
    let mut line: Vec<Letter> = Vec::new();
    let mut cut: Option<usize> = None;
    for letter in letters {
        let space = letter.character == ' ';
        // What's carried over after a cut may itself be too wide, so this goes round again.
        while !space && !line.is_empty() && advance(&line) + letter.advance > width {
            // A cut with nothing but spaces before it is no place to break.
            let at = cut
                .filter(|&at| line[..at].iter().any(|letter| letter.character != ' '))
                .unwrap_or(line.len());
            let rest = line.split_off(at);
            trim(&mut line);
            lines.push(std::mem::take(&mut line));
            line = rest
                .into_iter()
                .skip_while(|letter| letter.character == ' ')
                .collect();
            cut = None;
        }
        if wide(letter.character) && !line.is_empty() {
            cut = Some(line.len());
        }
        line.push(letter);
        if space || wide(letter.character) {
            cut = Some(line.len());
        }
    }
    trim(&mut line);
    lines.push(line);
}

/// White words in their coverage, with mipmaps for when the layer is drawn small.
fn texture(width: u32, height: u32, coverage: Vec<u8>) -> Texture {
    let mut levels = vec![(width, height, coverage)];
    while let Some((width, height, pixels)) = levels.last()
        && (*width > 1 || *height > 1)
    {
        levels.push(half(*width, *height, pixels));
    }
    let images = vec![
        levels
            .into_iter()
            .map(|(width, height, coverage)| Mip {
                width,
                height,
                channels: Channels::Rg,
                pixels: coverage
                    .into_iter()
                    .flat_map(|cover| [255, cover])
                    .collect(),
            })
            .collect(),
    ];
    Texture {
        format: Format::Rg88,
        flags: CLAMP_UVS,
        width,
        height,
        image_width: width,
        image_height: height,
        images,
        frames: Vec::new(),
    }
}

fn half(width: u32, height: u32, pixels: &[u8]) -> (u32, u32, Vec<u8>) {
    let (half_width, half_height) = ((width / 2).max(1), (height / 2).max(1));
    let at =
        |x: u32, y: u32| u32::from(pixels[(y.min(height - 1) * width + x.min(width - 1)) as usize]);
    let mut out = Vec::with_capacity((half_width * half_height) as usize);
    for y in 0..half_height {
        for x in 0..half_width {
            let (x, y) = (2 * x, 2 * y);
            let sum = at(x, y) + at(x + 1, y) + at(x, y + 1) + at(x + 1, y + 1);
            out.push(((sum + 2) / 4) as u8);
        }
    }
    (half_width, half_height, out)
}

#[cfg(test)]
mod tests;
