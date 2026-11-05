use super::fail;
use crate::Error;
use serde_json::{Map, Value};
use std::collections::HashSet;

pub(super) struct Token<'a> {
    pub(super) text: &'a str,
    pub(super) start: usize,
    pub(super) end: usize,
}

pub(super) fn bind(
    source: &str,
    constants: &Map<String, Value>,
    optional: Option<u8>,
) -> Result<(String, u8, u8, bool), Error> {
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
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if token.text.starts_with("apolline_")
            || matches!(
                token.text,
                "gl_FragData" | "gl_FragDepth" | "layout" | "attribute" | "out"
            )
        {
            return Err(fail(format!(
                "unsupported or reserved identifier {}",
                token.text
            )));
        }
        if token.text != "uniform" && token.text != "varying" {
            index += 1;
            continue;
        }
        let declaration = tokens
            .get(index..index + 4)
            .ok_or_else(|| fail("incomplete shader declaration"))?;
        let kind = declaration[1].text;
        let name = declaration[2].text;
        if declaration[3].text != ";" || !super::identifier(name) || !seen.insert(name) {
            return Err(fail(
                "arrays, duplicate declarations and complex uniforms are unsupported",
            ));
        }
        index += 4;
        if token.text == "varying" {
            let size = match (name, kind) {
                ("v_TexCoord", "vec2") => 2,
                ("v_TexCoord", "vec3") => 3,
                ("v_TexCoord", "vec4") => 4,
                _ => return Err(fail(format!("unsupported varying {name}"))),
            };
            texcoord = Some(size);
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
                        if tokens.iter().filter(|token| token.text == name).count() != 1 {
                            return Err(fail(format!(
                                "unbound optional texture is used by active code: {name}"
                            )));
                        }
                        replacements.push((token.start, declaration[3].end, String::new()));
                    } else {
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
        let neutral = match (name, kind) {
            ("g_Color", "vec3") => Some("vec3(1.0)"),
            ("g_Color4", "vec4") => Some("vec4(1.0)"),
            ("g_Alpha", "float") => Some("1.0"),
            _ => None,
        };
        let value = if let Some(neutral) = neutral {
            neutral.to_owned()
        } else {
            if name.starts_with("g_") {
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
            literal(kind, raw).map_err(|error| fail(format!("uniform {name}: {error}")))?
        };
        replacements.push((
            token.start,
            declaration[3].end,
            format!("const {kind} {name} = {value};"),
        ));
    }
    if let Some(unused) = constants.keys().find(|key| !used.contains(key.as_str())) {
        return Err(fail(format!("unknown material constant {unused}")));
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
    Ok((output, texcoord.unwrap_or(2), inputs, animated))
}

pub(super) fn annotation(source: &str, end: usize, name: &str) -> Result<Value, Error> {
    let tail = source[end..].split('\n').next().unwrap_or("").trim();
    match tail
        .strip_prefix("//")
        .map(str::trim)
        .filter(|text| text.starts_with('{'))
    {
        Some(json) => {
            serde_json::from_str(json).map_err(|error| fail(format!("uniform {name}: {error}")))
        }
        None => Ok(Value::Null),
    }
}

fn literal(kind: &str, value: &Value) -> Result<String, Error> {
    let value = crate::value::plain(value);
    if kind == "bool" {
        return value
            .as_bool()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .map(|value| value.to_string())
            .ok_or_else(|| fail("expected a boolean"));
    }
    if kind == "int" {
        return value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
            .and_then(|value| i32::try_from(value).ok())
            .map(|value| value.to_string())
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
        text.split_whitespace()
            .take(count + 1)
            .map(|word| word.parse().map_err(|_| fail("invalid numeric component")))
            .collect::<Result<_, _>>()?
    } else {
        vec![value.as_f64().ok_or_else(|| fail("expected a number"))?]
    };
    if numbers.len() != count || numbers.iter().any(|value| !(*value as f32).is_finite()) {
        return Err(fail(format!("expected {count} finite numeric components")));
    }
    let values = numbers
        .iter()
        .map(|value| format!("{:.9e}", *value as f32))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(if count == 1 {
        values
    } else {
        format!("{kind}({values})")
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
