use super::fail;
use crate::Error;
use serde_json::{Map, Value};
use std::collections::HashSet;

/// A material value the program reads as a uniform, set for each layer that draws with it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Constant {
    Bool(bool),
    Int(i32),
    /// One to four components, kept as bits so prepared fragments can be compared.
    Float {
        bits: [u32; 4],
        count: u8,
    },
}

impl Constant {
    pub fn floats(&self) -> [f32; 4] {
        match self {
            Constant::Float { bits, .. } => bits.map(f32::from_bits),
            Constant::Int(value) => [*value as f32, 0.0, 0.0, 0.0],
            Constant::Bool(value) => [f32::from(u8::from(*value)), 0.0, 0.0, 0.0],
        }
    }

    /// The same kind of value with new numbers, as many as it takes; numbers that aren't
    /// finite keep the old ones.
    pub fn with(&self, numbers: [f32; 4]) -> Constant {
        let old = self.floats();
        let numbers: [f32; 4] = std::array::from_fn(|at| {
            if numbers[at].is_finite() {
                numbers[at]
            } else {
                old[at]
            }
        });
        match self {
            Constant::Float { count, .. } => Constant::Float {
                bits: std::array::from_fn(|at| {
                    if at < usize::from(*count) {
                        numbers[at].to_bits()
                    } else {
                        0
                    }
                }),
                count: *count,
            },
            Constant::Int(_) => {
                Constant::Int(numbers[0].round().clamp(i32::MIN as f32, i32::MAX as f32) as i32)
            }
            Constant::Bool(_) => Constant::Bool(numbers[0] != 0.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    Vertex,
    Fragment,
}

/// What a shader reads from the engine beyond time, which the host has to supply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Reads {
    pub pointer: bool,
    pub parallax: bool,
    pub daytime: bool,
    pub audio: bool,
}

impl Reads {
    pub fn with(self, other: Reads) -> Reads {
        Reads {
            pointer: self.pointer || other.pointer,
            parallax: self.parallax || other.parallax,
            daytime: self.daytime || other.daytime,
            audio: self.audio || other.audio,
        }
    }
}

pub(super) struct Bound {
    pub body: String,
    pub texcoord: u8,
    pub inputs: u8,
    pub animated: bool,
    pub constants: Vec<(String, Constant)>,
    /// The material value each constant was given from, by the material's name for it.
    pub keys: Vec<Option<String>>,
    /// Material values this stage read, by the name the material gives them.
    pub used: HashSet<String>,
    /// Name and type of each varying, in order.
    pub varyings: Vec<(String, String)>,
    pub reads: Reads,
}

pub(super) struct Token<'a> {
    pub(super) text: &'a str,
    pub(super) start: usize,
    pub(super) end: usize,
}

pub(super) fn bind(
    source: &str,
    constants: &Map<String, Value>,
    optional: Option<u8>,
    stage: Stage,
) -> Result<Bound, Error> {
    if constants.len() > 128 || constants.keys().any(|key| key.len() > 128) {
        return Err(fail("too many or oversized material constants"));
    }
    let tokens = tokens(source)?;
    let mut replacements = Vec::new();
    let mut seen = HashSet::new();
    let mut used = HashSet::new();
    let mut texcoord = None;
    let mut inputs = 0u8;
    let mut animated = false;
    let mut bound = Vec::new();
    let mut keys = Vec::new();
    let mut varyings = Vec::new();
    let mut reads = Reads::default();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        // `out` is a helper function's parameter after `(` or `,`; anywhere else it would
        // declare a second output.
        let parameter = index > 0 && matches!(tokens[index - 1].text, "(" | ",");
        if token.text.starts_with("apolline_")
            || matches!(token.text, "gl_FragData" | "gl_FragDepth" | "layout")
            || (token.text == "attribute" && stage == Stage::Fragment)
            || (token.text == "gl_Position" && stage == Stage::Fragment)
            || (token.text == "gl_FragColor" && stage == Stage::Vertex)
            || (token.text == "out" && !parameter)
        {
            return Err(fail(format!(
                "unsupported or reserved identifier {}",
                token.text
            )));
        }
        if !matches!(token.text, "uniform" | "varying" | "attribute") {
            index += 1;
            continue;
        }
        let declaration = tokens
            .get(index..index + 4)
            .ok_or_else(|| fail("incomplete shader declaration"))?;
        let kind = declaration[1].text;
        let name = declaration[2].text;
        // The audio spectrum is the one array, by its own names and lengths.
        if token.text == "uniform" && kind == "float" && declaration[3].text == "[" {
            let rest = &tokens[index + 4..];
            let close = rest.iter().take(4).position(|token| token.text == "]");
            let length = close
                .filter(|close| *close > 0)
                .map(|close| &source[rest[0].start..rest[close - 1].end]);
            let ends = close.and_then(|close| rest.get(close + 1));
            if let (Some(bins), Some(length), Some(end)) = (spectrum(name), length, ends)
                && length == bins.to_string()
                && end.text == ";"
                && seen.insert(name)
            {
                reads.audio = true;
                index += 4 + close.unwrap_or(0) + 2;
                continue;
            }
        }
        // A varying may be an array, as blurs pass their taps: `varying vec2 v_TexCoord[13];`.
        // Its length comes digit by digit, as the tokens split numbers.
        let mut length = None;
        let mut ends = 4;
        if token.text == "varying" && declaration[3].text == "[" {
            let rest = &tokens[index + 4..];
            let close = rest.iter().take(3).position(|token| token.text == "]");
            let digits: Option<String> =
                close.map(|close| rest[..close].iter().map(|token| token.text).collect());
            if let (Some(close), Some(digits)) = (close, digits)
                && rest.get(close + 1).is_some_and(|end| end.text == ";")
                && let Ok(count) = digits.parse::<u8>()
                && (1..=32).contains(&count)
            {
                length = Some(count);
                ends = 4 + close + 2;
            }
        }
        if (length.is_none() && declaration[3].text != ";")
            || !super::identifier(name)
            || !seen.insert(name)
        {
            return Err(fail(
                "arrays, duplicate declarations and complex uniforms are unsupported",
            ));
        }
        index += ends;
        if token.text == "attribute" {
            // The host gives these as plain globals, set before the shader's own main.
            if !matches!(
                (name, kind),
                ("a_Position", "vec3") | ("a_TexCoord", "vec2")
            ) {
                return Err(fail(format!("unsupported attribute {name}")));
            }
            replacements.push((token.start, declaration[3].end, String::new()));
            continue;
        }
        if token.text == "varying" {
            let size = match kind {
                "float" => 1,
                "vec2" => 2,
                "vec3" => 3,
                "vec4" => 4,
                _ => return Err(fail(format!("unsupported varying type {kind}"))),
            };
            if name == "v_TexCoord" {
                texcoord = Some(size);
            }
            if name.starts_with("g_") || varyings.len() >= 16 {
                return Err(fail(format!("unsupported varying {name}")));
            }
            // An array's length is part of its type, which both stages have to agree on.
            let kind = match length {
                Some(count) => format!("{kind}[{count}]"),
                None => kind.to_owned(),
            };
            varyings.push((name.to_owned(), kind));
            continue;
        }
        let annotation = annotation(source, declaration[3].end, name)?;
        if name.starts_with("g_Texture")
            && kind == "sampler2D"
            && annotation.get("combo").is_some()
            && optional.is_none()
        {
            return Err(fail("texture-driven combos need another renderer"));
        }
        if stage == Stage::Vertex && kind == "sampler2D" {
            return Err(fail(
                "vertex shaders that sample textures need another renderer",
            ));
        }
        // How far textures were shrunk to save memory, which ours never are.
        if kind == "float" && name == "g_TextureReductionScale" {
            let whole = format!("const float {name} = 1.0;");
            replacements.push((token.start, declaration[3].end, whole));
            continue;
        }
        if let Some(suffix) = name.strip_prefix("g_Texture") {
            let bytes = suffix.as_bytes();
            if bytes
                .first()
                .is_some_and(|byte| (b'0'..=b'7').contains(byte))
            {
                let index = bytes[0] - b'0';
                let rest = &suffix[1..];
                if (rest.is_empty() && kind == "sampler2D")
                    || (rest == "Resolution" && kind == "vec4")
                {
                    if optional.is_some_and(|missing| missing & (1 << index) != 0) {
                        let read = tokens.iter().filter(|token| token.text == name).count() != 1;
                        // A texture that isn't there can't be read, but its size can: that of
                        // one pixel, which only places coordinates nothing then samples with.
                        let stand_in = match kind {
                            "vec4" if read => format!("const vec4 {name} = vec4(1.0);"),
                            _ if read => {
                                return Err(fail(format!(
                                    "unbound optional texture is used by active code: {name}"
                                )));
                            }
                            _ => String::new(),
                        };
                        replacements.push((token.start, declaration[3].end, stand_in));
                    } else if tokens.iter().filter(|token| token.text == name).count() > 1 {
                        // Only a slot something reads needs binding, not one just declared.
                        inputs |= 1 << index;
                    }
                    continue;
                }
            }
            return Err(fail(format!("unsupported texture uniform {name}")));
        }
        if kind == "sampler2D" {
            return Err(fail("samplers must be named g_Texture0 through g_Texture7"));
        }
        if stage == Stage::Vertex && name == "g_ModelViewProjectionMatrix" && kind == "mat4" {
            continue;
        }
        // Where the layer is on screen, turned around: from the screen back to the pass's
        // own units, or to its picture's.
        if kind == "mat4"
            && matches!(
                name,
                "g_ModelViewProjectionMatrixInverse" | "g_EffectTextureProjectionMatrixInverse"
            )
        {
            continue;
        }
        if kind == "float" && matches!(name, "g_Time" | "g_Frametime") {
            animated = true;
            continue;
        }
        if matches!(
            (name, kind),
            ("g_Screen", "vec3") | ("g_TexelSize" | "g_TexelSizeHalf", "vec2")
        ) {
            continue;
        }
        let input = match (name, kind) {
            ("g_PointerPosition", "vec2") => Some(&mut reads.pointer),
            ("g_ParallaxPosition", "vec2") => Some(&mut reads.parallax),
            ("g_Daytime", "float") => Some(&mut reads.daytime),
            _ => None,
        };
        if let Some(read) = input {
            *read = true;
            continue;
        }
        let neutral = match (name, kind) {
            ("g_Color", "vec3") => Some("vec3(1.0)"),
            ("g_Color4", "vec4") => Some("vec4(1.0)"),
            ("g_Alpha", "float") => Some("1.0"),
            _ => None,
        };
        if let Some(neutral) = neutral {
            replacements.push((
                token.start,
                declaration[3].end,
                format!("const {kind} {name} = {neutral};"),
            ));
            continue;
        }
        // Stock shaders name their own values g_ too; the annotation ties those to the
        // material, and anything else with the prefix comes from the engine.
        if name.starts_with("g_") && annotation.get("material").is_none() {
            return Err(fail(format!("dynamic or unsupported uniform {name}")));
        }
        let material = annotation
            .get("material")
            .and_then(Value::as_str)
            .unwrap_or(name);
        let (key, supplied) = if let Some(value) = constants.get(material) {
            (material, Some(value))
        } else {
            (name, constants.get(name))
        };
        let raw = supplied
            .or_else(|| annotation.get("default"))
            .ok_or_else(|| fail(format!("uniform {name} needs a material value or default")))?;
        if supplied.is_some() {
            used.insert(key.to_owned());
        }
        let constant =
            constant(kind, raw).map_err(|error| fail(format!("uniform {name}: {error}")))?;
        bound.push((name.to_owned(), constant));
        keys.push(supplied.is_some().then(|| key.to_owned()));
    }
    let mut output = String::new();
    let mut from = 0;
    for (start, end, replacement) in replacements {
        output.push_str(&source[from..start]);
        output.push_str(&replacement);
        for _ in source[start..end].bytes().filter(|byte| *byte == b'\n') {
            output.push('\n');
        }
        from = end;
    }
    output.push_str(&source[from..]);
    Ok(Bound {
        body: output,
        texcoord: texcoord.unwrap_or(2),
        inputs,
        animated,
        constants: bound,
        keys,
        used,
        varyings,
        reads,
    })
}

pub(super) fn annotation(source: &str, end: usize, name: &str) -> Result<Value, Error> {
    let tail = source[end..].split('\n').next().unwrap_or("").trim();
    match tail
        .strip_prefix("//")
        .map(str::trim)
        .filter(|text| text.starts_with('{'))
    {
        // The editor reads numbers written with leading zeros, `[0,01]`; JSON doesn't.
        Some(json) => serde_json::from_str(json)
            .or_else(|error| serde_json::from_str(&unpadded(json)).map_err(|_| error))
            .map_err(|error| fail(format!("uniform {name}: {error}"))),
        None => Ok(Value::Null),
    }
}

/// `json` with the leading zeros of its numbers outside strings dropped.
fn unpadded(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let mut quoted = false;
    let mut escaped = false;
    let mut previous = ' ';
    let mut chars = json.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
        } else if c == '"' {
            quoted = true;
        } else if c == '0'
            && !previous.is_ascii_digit()
            && previous != '.'
            && chars.peek().is_some_and(char::is_ascii_digit)
        {
            continue;
        }
        out.push(c);
        previous = c;
    }
    out
}

fn constant(kind: &str, value: &Value) -> Result<Constant, Error> {
    let value = crate::value::plain(value);
    if kind == "bool" {
        return value
            .as_bool()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .map(Constant::Bool)
            .ok_or_else(|| fail("expected a boolean"));
    }
    if kind == "int" {
        return value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .and_then(|value| i32::try_from(value).ok())
            .map(Constant::Int)
            .ok_or_else(|| fail("expected a signed integer"));
    }
    let count = match kind {
        "float" => 1,
        "vec2" => 2,
        "vec3" => 3,
        "vec4" => 4,
        _ => return Err(fail(format!("unsupported uniform type {kind}"))),
    };
    let numbers: Vec<f64> = if let Some(values) = value.as_array() {
        if values.len() != count {
            return Err(fail(format!("expected {count} numeric components")));
        }
        values
            .iter()
            .map(|value| {
                value
                    .as_f64()
                    .ok_or_else(|| fail("expected numeric components"))
            })
            .collect::<Result<_, _>>()?
    } else if let Some(text) = value.as_str() {
        // Components apart by spaces, or commas as some workshop shaders write them.
        text.split(|c: char| c.is_whitespace() || c == ',')
            .filter(|word| !word.is_empty())
            .take(count + 1)
            .map(|word| word.parse().map_err(|_| fail("invalid numeric component")))
            .collect::<Result<_, _>>()?
    } else {
        vec![value.as_f64().ok_or_else(|| fail("expected a number"))?]
    };
    if numbers.len() != count || numbers.iter().any(|value| !(*value as f32).is_finite()) {
        return Err(fail(format!("expected {count} finite numeric components")));
    }
    let mut bits = [0; 4];
    for (bit, number) in bits.iter_mut().zip(&numbers) {
        *bit = (*number as f32).to_bits();
    }
    Ok(Constant::Float {
        bits,
        count: count as u8,
    })
}

pub(super) fn tokens(source: &str) -> Result<Vec<Token<'_>>, Error> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"//") {
            at += source[at..].find('\n').unwrap_or(bytes.len() - at);
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            let end = source[at + 2..]
                .find("*/")
                .ok_or_else(|| fail("unclosed block comment"))?;
            at += end + 4;
            continue;
        }
        let start = at;
        if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
        } else {
            if !bytes[at].is_ascii() || matches!(bytes[at], b'"' | b'\\') {
                return Err(fail("unsupported shader token"));
            }
            at += 1;
        }
        if &source[start..at] == "#"
            && tokens
                .last()
                .is_some_and(|token: &Token<'_>| token.text == "#")
        {
            return Err(fail("shader token pasting is unsupported"));
        }
        tokens.push(Token {
            text: &source[start..at],
            start,
            end: at,
        });
    }
    Ok(tokens)
}

/// How many bins an audio spectrum uniform holds, by its name.
fn spectrum(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("g_AudioSpectrum")?;
    let bins = rest
        .strip_suffix("Left")
        .or_else(|| rest.strip_suffix("Right"))?;
    match bins {
        "16" => Some(16),
        "32" => Some(32),
        "64" => Some(64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::Constant;

    #[test]
    fn new_numbers_keep_a_constants_kind() {
        let colour = Constant::Float {
            bits: [1.0f32, 2.0, 3.0, 0.0].map(f32::to_bits),
            count: 3,
        };
        assert_eq!(
            colour.with([4.0, f32::NAN, 6.0, 7.0]).floats(),
            [4.0, 2.0, 6.0, 0.0]
        );
        assert_eq!(
            Constant::Int(1).with([2.6, 0.0, 0.0, 0.0]),
            Constant::Int(3)
        );
        assert_eq!(Constant::Bool(true).with([0.0; 4]), Constant::Bool(false));
    }

    #[test]
    fn padded_numbers_read_as_the_editor_reads_them() {
        let source = "uniform float u_Speed; // {\"material\":\"Speed\",\"default\":1,\"range\":[0,01],\"label\":\"a \\\"0,01\\\" b\"}";
        let end = source.find(';').unwrap() + 1;
        let read = super::annotation(source, end, "u_Speed").unwrap();
        assert_eq!(read["range"], serde_json::json!([0, 1]));
        assert_eq!(read["label"], "a \"0,01\" b");
        assert_eq!(super::unpadded("[0.05, 007, 10, 0]"), "[0.05, 7, 10, 0]");
    }
}
