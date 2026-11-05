use super::{
    expression::{self, Macros, Token},
    fail,
};
use crate::Error;
use std::collections::BTreeMap;

struct Branch {
    parent: bool,
    taken: bool,
    active: bool,
    otherwise: bool,
}

/// Selects branches before uniform introspection; the driver receives the same selected code.
pub(super) fn select(source: &str, combos: &BTreeMap<String, i32>) -> Result<String, Error> {
    let mut macros = Macros::new();
    for line in super::COMPATIBILITY
        .lines()
        .filter_map(|line| line.strip_prefix("#define "))
    {
        let name = line
            .split(|c: char| c.is_ascii_whitespace() || c == '(')
            .next()
            .unwrap_or("");
        macros.insert(name.into(), None);
    }
    macros.insert("GLSL".into(), Some(vec![Token::Number(1)]));
    for (name, value) in combos {
        macros.insert(name.clone(), Some(vec![Token::Number(*value)]));
    }
    let mut branches: Vec<Branch> = Vec::new();
    let mut output = String::with_capacity(source.len());
    let mut blocked = false;
    let mut work = 65536;
    for original in source.lines() {
        let visible = super::visible_line(original, &mut blocked);
        let active = branches.last().is_none_or(|branch| branch.active);
        let Some(directive) = visible.trim().strip_prefix('#').map(str::trim_start) else {
            if active {
                output.push_str(original);
            }
            output.push('\n');
            continue;
        };
        if visible.trim_end().ends_with('\\') {
            return Err(fail("continued preprocessor directives are unsupported"));
        }
        let (command, rest) = directive
            .split_once(char::is_whitespace)
            .unwrap_or((directive, ""));
        let rest = rest.trim();
        let mut emit = false;
        match command {
            "if" | "ifdef" | "ifndef" => {
                if branches.len() >= 64 {
                    return Err(fail("shader conditionals exceed 64 levels"));
                }
                let selected = if !active {
                    false
                } else {
                    match command {
                        "if" => expression::condition(rest, &macros, &mut work)?,
                        _ => {
                            let name = macro_name(rest)?;
                            expression::condition(
                                &format!(
                                    "{}defined({name})",
                                    if command == "ifndef" { "!" } else { "" }
                                ),
                                &macros,
                                &mut work,
                            )?
                        }
                    }
                };
                branches.push(Branch {
                    parent: active,
                    taken: selected,
                    active: selected,
                    otherwise: false,
                });
            }
            "elif" => {
                let branch = branches
                    .last_mut()
                    .ok_or_else(|| fail("#elif without #if"))?;
                if branch.otherwise {
                    return Err(fail("#elif after #else"));
                }
                branch.active = branch.parent
                    && !branch.taken
                    && expression::condition(rest, &macros, &mut work)?;
                branch.taken |= branch.active;
            }
            "else" => {
                empty(rest)?;
                let branch = branches
                    .last_mut()
                    .ok_or_else(|| fail("#else without #if"))?;
                if branch.otherwise {
                    return Err(fail("duplicate #else"));
                }
                branch.otherwise = true;
                branch.active = branch.parent && !branch.taken;
                branch.taken = true;
            }
            "endif" => {
                empty(rest)?;
                branches.pop().ok_or_else(|| fail("#endif without #if"))?;
            }
            "define" if active => {
                let end = rest
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(rest.len());
                let name = macro_name(&rest[..end])?;
                if super::protected(name) || combos.contains_key(name) {
                    return Err(fail("shader redefines a host-controlled macro"));
                }
                if macros.contains_key(name) {
                    return Err(fail("duplicate active macro definition"));
                }
                if macros.len() >= 256 {
                    return Err(fail("shader exceeds 256 macros"));
                }
                // Function and noninteger macros remain intact for the driver; conditions
                // using their values are refused rather than guessed.
                let body = if rest[end..].starts_with('(') {
                    None
                } else {
                    expression::tokens(rest[end..].trim()).ok()
                };
                macros.insert(name.into(), body);
                emit = true;
            }
            "undef" if active => {
                let name = macro_name(rest)?;
                if super::protected(name) || combos.contains_key(name) {
                    return Err(fail("shader undefines a host-controlled macro"));
                }
                macros.remove(name);
                emit = true;
            }
            "line" => emit = true,
            "error" if active => return Err(fail("active shader #error directive")),
            "" => {}
            _ if !active => {}
            _ => return Err(fail(format!("unsupported shader directive #{command}"))),
        }
        if emit {
            output.push_str(original);
        }
        output.push('\n');
    }
    if !branches.is_empty() {
        return Err(fail("unclosed shader conditional"));
    }
    if blocked {
        return Err(fail("unclosed shader comment"));
    }
    Ok(output)
}

fn macro_name(name: &str) -> Result<&str, Error> {
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        })
    {
        Err(fail("invalid preprocessor macro name"))
    } else {
        Ok(name)
    }
}

fn empty(rest: &str) -> Result<(), Error> {
    if rest.is_empty() {
        Ok(())
    } else {
        Err(fail("unexpected tokens after conditional directive"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_branches_follow_combo_overrides_and_preserve_line_count() {
        let source = "#if MODE == 1\nfirst\n#elif MODE == 2\n#ifdef GLSL\nsecond\n#endif\n#else\nthird\n#endif\n";
        for (mode, word) in [(0, "third"), (1, "first"), (2, "second")] {
            let output = select(source, &BTreeMap::from([("MODE".into(), mode)])).unwrap();
            assert_eq!(
                output
                    .lines()
                    .filter(|line| !line.is_empty())
                    .collect::<Vec<_>>(),
                [word]
            );
            assert_eq!(output.lines().count(), source.lines().count());
        }
    }

    #[test]
    fn include_guards_and_local_macros_obey_active_branch_state() {
        let source = "#ifndef HEADER\n#define HEADER\n#define MODE 1 + 2\n#endif\n#ifdef HEADER\n#if MODE * 3 == 7\nyes\n#else\nwrong\n#endif\n#endif\n#undef MODE\n#if MODE\nwrong\n#endif\n#ifndef HLSL\nglsl\n#endif\n";
        let output = select(source, &BTreeMap::new()).unwrap();
        assert!(output.contains("yes"));
        assert!(output.contains("glsl"));
        assert!(!output.contains("wrong"));
        let output = select("#if 0\n#define MODE 1\n#if INVALID(\nwrong\n#endif\n#endif\n#ifdef MODE\nwrong\n#endif", &BTreeMap::new()).unwrap();
        assert!(!output.contains("wrong"));
    }

    #[test]
    fn malformed_or_hostile_directives_fail_explicitly() {
        for source in [
            "#else",
            "#endif",
            "#elif 1",
            "#if 1",
            "#if 1\n#else\n#else\n#endif",
            "#if 1\n#else\n#elif 1\n#endif",
            "#if 1\n#endif trailing",
            "#define GLSL 0",
            "#undef GLSL",
            "#define FOO 1\n#define FOO 2",
            "#pragma mystery",
            "#error broken",
            "#if defined(GL_ES)\n#endif",
            "#define X 1 + \\\n2",
            "#define 123 1",
        ] {
            assert!(select(source, &BTreeMap::new()).is_err(), "{source}");
        }
        let nested = format!("{}{}", "#if 1\n".repeat(65), "#endif\n".repeat(65));
        assert!(select(&nested, &BTreeMap::new()).is_err());
    }
}
