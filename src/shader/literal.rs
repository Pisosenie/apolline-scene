//! Wallpaper Engine's shaders are written to pass as HLSL too, where `max(0, color)` is
//! fine; GLSL has `max(color, 0.0)` but no `max` that takes the number first. The two orders
//! give the same, so a `max` or `min` that starts with a number is turned around.

use super::parameters::{Token, tokens};
use crate::Error;

/// The most calls turned around in one stage.
const MOST: usize = 256;

/// Where a call that starts with a number has its parts in the source.
struct Call {
    /// The number's bytes.
    number: (usize, usize),
    /// Where what follows the comma starts, and where the closing bracket is.
    rest: usize,
    close: usize,
}

pub(super) fn turned(source: &str) -> Result<String, Error> {
    let mut source = source.to_owned();
    for _ in 0..MOST {
        let Some(call) = first(&tokens(&source)?) else {
            break;
        };
        let number = &source[call.number.0..call.number.1];
        let number = if number.contains('.') {
            number.to_owned()
        } else {
            format!("{number}.0")
        };
        let rest = source[call.rest..call.close].trim_end().to_owned();
        source.replace_range(call.number.0..call.close, &format!("{rest}, {number}"));
    }
    Ok(source)
}

/// The first `max(number, …)` or `min(number, …)`.
fn first(tokens: &[Token]) -> Option<Call> {
    (0..tokens.len()).find_map(|at| {
        if !matches!(tokens[at].text, "max" | "min") || tokens.get(at + 1)?.text != "(" {
            return None;
        }
        // A number is digits and a point written together.
        let after = &tokens[at + 2..];
        let mut end = 0;
        while after.get(end).is_some_and(|token| {
            token.text.chars().all(|c| c.is_ascii_digit() || c == '.')
                && (end == 0 || after[end - 1].end == token.start)
        }) {
            end += 1;
        }
        if !after[..end].iter().any(|token| token.text != ".") || after.get(end)?.text != "," {
            return None;
        }
        let rest = after.get(end + 1)?;
        let mut depth = 0usize;
        let close = after[end + 1..].iter().find(|token| match token.text {
            "(" | "[" => {
                depth += 1;
                false
            }
            ")" | "]" if depth == 0 => true,
            ")" | "]" => {
                depth -= 1;
                false
            }
            _ => false,
        })?;
        (close.text == ")" && close.start > rest.start).then_some(Call {
            number: (after[0].start, after[end - 1].end),
            rest: rest.start,
            close: close.start,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::turned;

    #[test]
    fn a_max_or_min_that_starts_with_a_number_is_turned_around() {
        assert_eq!(
            turned("c = vec4(max(0, a.rgb), 1.0);").unwrap(),
            "c = vec4(max(a.rgb, 0.0), 1.0);"
        );
        assert_eq!(
            turned("d = 1.0 - max(0, min(1, f(d, 2)));").unwrap(),
            "d = 1.0 - max(min(f(d, 2), 1.0), 0.0);"
        );
        assert_eq!(
            turned("x = min(0.5, y[1]) + max(a, 0);").unwrap(),
            "x = min(y[1], 0.5) + max(a, 0);"
        );
    }
}
