use super::{fail, parameters};
use crate::Error;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

pub(super) struct Options {
    pub missing: u8,
    pub controlled: HashSet<String>,
}

pub(super) fn fill_defaults(source: &str, names: &mut Vec<Option<String>>) -> Result<(), Error> {
    if names.len() > 8 {
        return Err(fail("material exceeds eight texture slots"));
    }
    let supplied = names.iter().enumerate().fold(0u8, |mask, (slot, name)| {
        mask | if name.is_some() { 1 << slot } else { 0 }
    });
    let tokens = parameters::tokens(source)?;
    let mut defaults = 0u8;
    for declaration in tokens.windows(4) {
        if declaration[0].text != "uniform"
            || declaration[1].text != "sampler2D"
            || declaration[3].text != ";"
        {
            continue;
        }
        let name = declaration[2].text;
        let slot = slot(name)?;
        if supplied & (1 << slot) != 0 {
            continue;
        }
        let annotation = parameters::annotation(source, declaration[3].end, name)?;
        let resource = match annotation.get("default") {
            None | Some(Value::Null) => continue,
            Some(Value::String(resource)) if resource.is_empty() => continue,
            Some(Value::String(resource)) if resource.len() <= 4082 => resource,
            _ => {
                return Err(fail(
                    "sampler default must be a bounded texture resource name",
                ));
            }
        };
        if defaults & (1 << slot) != 0 {
            if names[slot as usize].as_deref() == Some(resource) {
                continue;
            }
            return Err(fail("ambiguous defaults for the same sampler slot"));
        }
        defaults |= 1 << slot;
        names.resize_with(names.len().max(usize::from(slot) + 1), || None);
        names[slot as usize] = Some(resource.clone());
    }
    Ok(())
}

fn slot(name: &str) -> Result<u8, Error> {
    name.strip_prefix("g_Texture")
        .filter(|index| index.len() == 1)
        .and_then(|index| index.parse::<u8>().ok())
        .filter(|slot| *slot < 8)
        .ok_or_else(|| fail("samplers must be named g_Texture0 through g_Texture7"))
}

/// Derives optional-sampler combos from bindings, before conditional selection.
pub(super) fn resolve(
    source: &str,
    bindings: u8,
    supplied: &Map<String, Value>,
    combos: &mut BTreeMap<String, i32>,
) -> Result<Options, Error> {
    let tokens = parameters::tokens(source)?;
    let declared = combos.len();
    let mut seen = HashMap::new();
    let mut slots = 0u8;
    let mut missing = 0u8;
    for declaration in tokens.windows(4) {
        if declaration[0].text != "uniform"
            || declaration[1].text != "sampler2D"
            || declaration[3].text != ";"
        {
            continue;
        }
        let name = declaration[2].text;
        let annotation = parameters::annotation(source, declaration[3].end, name)?;
        let Some(combo) = annotation.get("combo") else {
            continue;
        };
        let combo = combo
            .as_str()
            .filter(|name| super::identifier(name) && !super::protected(name))
            .ok_or_else(|| fail("invalid texture-driven combo name"))?;
        let slot = slot(name)?;
        if let Some((old_slot, old_annotation)) = seen.get(combo) {
            if *old_slot == slot && old_annotation == &annotation {
                continue;
            }
            return Err(fail("duplicate texture-driven combo or sampler slot"));
        }
        if slots & (1 << slot) != 0 {
            return Err(fail("duplicate texture-driven combo or sampler slot"));
        }
        seen.insert(combo.to_owned(), (slot, annotation.clone()));
        slots |= 1 << slot;
        if combos.contains_key(combo) {
            return Err(fail("combo is controlled by both metadata and a texture"));
        }
        let bound = bindings & (1 << slot) != 0;
        if !bound {
            if annotation
                .get("default")
                .is_some_and(|value| !value.is_null() && value.as_str() != Some(""))
            {
                return Err(fail(
                    "unbound optional texture has a default resource; bind it explicitly",
                ));
            }
            missing |= 1 << slot;
        }
        let value = i32::from(bound);
        if let Some(supplied) = supplied.get(combo)
            && super::integer(supplied)? != value
        {
            return Err(fail(format!(
                "texture-driven combo {combo} conflicts with actual bindings"
            )));
        }
        let painted = match annotation.get("mode").and_then(Value::as_str) {
            None if annotation.get("mode").is_none() => false,
            Some("opacitymask" | "rgbmask" | "flowmask") => true,
            _ => return Err(fail("unsupported optional texture mode")),
        };
        if declared + seen.len() > 64 {
            return Err(fail("more than 64 combos"));
        }
        if bound || painted {
            combos.insert(combo.into(), value);
        }
    }
    Ok(Options {
        missing,
        controlled: seen.into_keys().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::Fragment;
    use serde_json::json;

    fn prepare(source: &str, bindings: u8, combos: Value) -> Result<Fragment, Error> {
        Fragment::prepare_bound(
            "mask.frag",
            combos.as_object().unwrap(),
            &Map::new(),
            bindings,
            |_| Ok(Some(source.into())),
        )
    }

    const OPTIONAL: &str = "uniform sampler2D g_Texture0;\nuniform sampler2D g_Texture7; // {\"combo\":\"MASK\"}\nuniform vec4 g_Texture7Resolution;\n#if MASK\nuniform float g_Time;\n#endif\nvoid main() {\nvec4 pixel = texSample2D(g_Texture0, vec2(0.5));\n#if MASK\npixel.a *= texSample2D(g_Texture7, vec2(0.5)).r * g_Texture7Resolution.x * g_Time;\n#endif\ngl_FragColor = pixel;\n}";

    #[test]
    fn defaults_fill_missing_slots_and_keep_explicit_overrides() {
        let source = "uniform sampler2D g_Texture0; // {\"default\":\"util/white\"}\nuniform sampler2D g_Texture7; // {\"default\":\"util/noise\",\"combo\":\"MASK\"}";
        let mut names = Vec::new();
        fill_defaults(source, &mut names).unwrap();
        assert_eq!(names.len(), 8);
        assert_eq!(names[0].as_deref(), Some("util/white"));
        assert_eq!(names[7].as_deref(), Some("util/noise"));
        assert!(names[1..7].iter().all(Option::is_none));
        names[0] = Some("override".into());
        fill_defaults(source, &mut names).unwrap();
        assert_eq!(names[0].as_deref(), Some("override"));
    }

    #[test]
    fn default_metadata_cannot_overflow_slots_or_choose_an_ambiguous_resource() {
        for source in [
            "uniform sampler2D g_Texture8; // {\"default\":\"x\"}",
            "uniform sampler2D g_Texture1; // {\"default\":true}",
            "uniform sampler2D g_Texture1; // {\"default\":\"a\"}\nuniform sampler2D g_Texture1; // {\"default\":\"b\"}",
        ] {
            assert!(fill_defaults(source, &mut Vec::new()).is_err(), "{source}");
        }
        assert!(fill_defaults("", &mut vec![None; 9]).is_err());
        assert!(
            fill_defaults(
                &format!(
                    "uniform sampler2D g_Texture1; // {{\"default\":\"{}\"}}",
                    "x".repeat(4083)
                ),
                &mut Vec::new()
            )
            .is_err()
        );
        let mut overridden = vec![None, Some("explicit".into())];
        fill_defaults(
            "uniform sampler2D g_Texture1; // {\"default\":true}",
            &mut overridden,
        )
        .unwrap();
        assert_eq!(overridden[1].as_deref(), Some("explicit"));
    }

    #[test]
    fn guarded_metadata_is_read_once_and_reused_before_specialization() {
        let mut reads = 0;
        let template = crate::shader::Template::load("entry.frag", |name| {
            reads += 1;
            Ok(match name {
                "shaders/entry.frag" => Some("#include \"texture.h\"\n#include \"texture.h\"\nvoid main() { gl_FragColor = texSample2D(g_Texture0, vec2(0.5)); }".into()),
                "shaders/texture.h" => Some("#ifndef TEX_HEADER\n#define TEX_HEADER\nuniform sampler2D g_Texture0; // {\"default\":\"base\",\"combo\":\"TEXTURE\"}\n#endif".into()),
                _ => None,
            })
        }).unwrap();
        let mut names = Vec::new();
        template.fill_defaults(&mut names).unwrap();
        let fragment = template.prepare(&Map::new(), &Map::new(), Some(1)).unwrap();
        assert_eq!(names, [Some("base".into())]);
        assert_eq!(reads, 2);
        assert!(fragment.source.contains("#define TEXTURE 1"));
        assert!(fragment.validate_inputs(1).is_ok());
    }

    #[test]
    fn texture_presence_selects_code_inputs_and_animation_without_dummy_textures() {
        let absent = prepare(OPTIONAL, 1, json!({})).unwrap();
        assert_eq!(absent.inputs, 1);
        assert!(!absent.animated);
        assert!(!absent.source.contains("#define MASK"));
        assert!(!absent.source.contains("g_Texture7"));
        assert!(absent.validate_inputs(1).is_ok());
        assert!(absent.validate_inputs(129).is_err());
        let present = prepare(OPTIONAL, 129, json!({"MASK":true})).unwrap();
        assert_eq!(present.inputs, 129);
        assert!(present.animated);
        assert!(present.source.contains("#define MASK 1"));
        assert!(present.validate_inputs(129).is_ok());
        assert!(present.validate_inputs(1).is_err());
    }

    #[test]
    fn contradictory_or_unbound_active_reads_cannot_silently_sample_old_gl_state() {
        assert!(prepare(OPTIONAL, 1, json!({"MASK":1})).is_err());
        assert!(prepare(OPTIONAL, 129, json!({"MASK":0})).is_err());
        let read = "uniform sampler2D g_Texture1; // {\"combo\":\"MASK\"}\nvoid main() { gl_FragColor = texSample2D(g_Texture1, vec2(0.5)); }";
        assert!(prepare(read, 1, json!({})).is_err());
        let resolution = "uniform sampler2D g_Texture1; // {\"combo\":\"MASK\"}\nuniform vec4 g_Texture1Resolution;\nvoid main() { gl_FragColor = g_Texture1Resolution; }";
        assert!(prepare(resolution, 1, json!({})).is_err());
        assert!(prepare(OPTIONAL, 0, json!({})).is_err());
    }

    #[test]
    fn ambiguous_metadata_and_unresolved_defaults_are_explicit_errors() {
        for source in [
            "uniform sampler2D g_Texture1; // {\"combo\":\"MASK\",\"default\":\"util/white\"}",
            "uniform sampler2D g_Texture1; // {\"combo\":\"MASK\"}\nuniform sampler2D g_Texture2; // {\"combo\":\"MASK\"}",
            "uniform sampler2D g_Texture1; // {\"combo\":\"g_Texture0\"}",
            "uniform sampler2D g_Texture1; // {\"combo\":false}",
            "// [COMBO] {\"combo\":\"MASK\",\"default\":0}\nuniform sampler2D g_Texture1; // {\"combo\":\"MASK\"}",
        ] {
            assert!(prepare(source, 1, json!({})).is_err(), "{source}");
        }
    }

    #[test]
    fn removing_multiline_optional_declarations_keeps_source_line_numbers() {
        let source = "uniform\nsampler2D\ng_Texture1; // {\"combo\":\"MASK\"}\nvoid main() { gl_FragColor = vec4(1); }";
        let absent = prepare(source, 1, json!({})).unwrap();
        assert!(absent.source.contains("#line 1 1\n\n\n //"));
    }

    #[test]
    fn macro_token_pasting_cannot_hide_uniforms_from_binding_validation() {
        let source = "#define JOIN(a,b) a ## b\nuniform sampler2D g_Texture7; // {\"combo\":\"MASK\"}\nJOIN(uni,form) sampler2D JOIN(g_Texture,7);\nvoid main() { gl_FragColor = texSample2D(JOIN(g_Texture,7), vec2(0.5)); }";
        let error = prepare(source, 1, json!({})).err().unwrap().to_string();
        assert!(error.contains("token pasting"));
    }

    #[test]
    fn plain_optional_toggle_is_absent_while_a_paint_combo_is_defined_as_zero() {
        let source = "uniform sampler2D g_Texture1; // {\"combo\":\"MASK\"}\n#ifdef MASK\nuniform float g_Time;\nvoid main() { gl_FragColor = vec4(g_Time); }\n#else\nvoid main() { gl_FragColor = vec4(1); }\n#endif";
        let optional = prepare(source, 1, json!({"MASK":0})).unwrap();
        assert!(!optional.animated);
        assert!(!optional.source.contains("#define MASK"));
        let painted = prepare(
            &source.replace(
                "\"combo\":\"MASK\"",
                "\"combo\":\"MASK\",\"mode\":\"opacitymask\"",
            ),
            1,
            json!({}),
        )
        .unwrap();
        assert!(painted.animated);
        assert!(painted.source.contains("#define MASK 0"));
    }
}
