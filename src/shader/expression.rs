use super::fail;
use crate::Error;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub(super) enum Token {
    Number(i32),
    Name(String),
    Op(&'static str),
}

pub(super) type Macros = HashMap<String, Option<Vec<Token>>>;

pub(super) fn tokens(source: &str) -> Result<Vec<Token>, Error> {
    let bytes = source.as_bytes();
    let mut at = 0;
    let mut output = Vec::new();
    while at < bytes.len() {
        if output.len() >= 1024 {
            return Err(fail("conditional expression exceeds 1024 tokens"));
        }
        let start = at;
        if bytes[at].is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
            at += 1;
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            output.push(Token::Name(source[start..at].into()));
        } else if bytes[at].is_ascii_digit() {
            at += 1;
            while at < bytes.len() && bytes[at].is_ascii_alphanumeric() {
                at += 1;
            }
            let text = &source[start..at];
            let number =
                if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                    i32::from_str_radix(hex, 16).ok()
                } else if text.len() > 1 && text.starts_with('0') {
                    // Do not silently interpret a C-style octal literal as decimal.
                    None
                } else {
                    text.parse::<i32>().ok()
                };
            output.push(Token::Number(
                number.ok_or_else(|| fail("unsupported conditional integer"))?,
            ));
        } else {
            let operator = [
                "&&", "||", "==", "!=", "<=", ">=", "<<", ">>", "(", ")", "!", "~", "+", "-", "*",
                "/", "%", "<", ">", "&", "|", "^",
            ]
            .into_iter()
            .find(|operator| source[at..].starts_with(operator))
            .ok_or_else(|| fail("unsupported conditional expression token"))?;
            at += operator.len();
            output.push(Token::Op(operator));
        }
    }
    Ok(output)
}

pub(super) fn condition(source: &str, macros: &Macros, budget: &mut usize) -> Result<bool, Error> {
    let input = tokens(source)?;
    let mut expanded = Vec::new();
    expand(&input, macros, &mut Vec::new(), &mut expanded, budget)?;
    let mut parser = Parser {
        tokens: &expanded,
        at: 0,
    };
    let value = parser.expression(0, true, 0)?;
    if parser.at != expanded.len() {
        return Err(fail("trailing conditional expression tokens"));
    }
    Ok(value != 0)
}

fn expand(
    input: &[Token],
    macros: &Macros,
    stack: &mut Vec<String>,
    output: &mut Vec<Token>,
    budget: &mut usize,
) -> Result<(), Error> {
    if stack.len() >= 16 {
        return Err(fail("conditional macro expansion exceeds 16 levels"));
    }
    let mut at = 0;
    while at < input.len() {
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| fail("conditional expression work budget exceeded"))?;
        if output.len() >= 4096 {
            return Err(fail("expanded condition exceeds 4096 tokens"));
        }
        match &input[at] {
            Token::Name(name) if name == "defined" => {
                at += 1;
                let parenthesis = matches!(input.get(at), Some(Token::Op("(")));
                at += usize::from(parenthesis);
                let Some(Token::Name(name)) = input.get(at) else {
                    return Err(fail("defined needs a macro name"));
                };
                platform_name(name)?;
                output.push(Token::Number(i32::from(macros.contains_key(name))));
                at += 1;
                if parenthesis {
                    if !matches!(input.get(at), Some(Token::Op(")"))) {
                        return Err(fail("defined needs a closing parenthesis"));
                    }
                    at += 1;
                }
                continue;
            }
            Token::Name(name) => {
                platform_name(name)?;
                match macros.get(name) {
                    Some(Some(body)) => {
                        if stack.contains(name) {
                            return Err(fail("recursive conditional macro"));
                        }
                        stack.push(name.clone());
                        expand(body, macros, stack, output, budget)?;
                        stack.pop();
                    }
                    Some(None) => return Err(fail("opaque or function macro used in a condition")),
                    None => output.push(Token::Number(0)),
                }
            }
            token => output.push(token.clone()),
        }
        at += 1;
    }
    Ok(())
}

fn platform_name(name: &str) -> Result<(), Error> {
    if name.starts_with("GL_") || name.starts_with("__") {
        Err(fail(
            "platform-dependent shader conditions need another renderer",
        ))
    } else {
        Ok(())
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
}

impl Parser<'_> {
    fn expression(&mut self, precedence: u8, evaluate: bool, depth: usize) -> Result<i32, Error> {
        if depth >= 64 {
            return Err(fail("conditional expression exceeds 64 levels"));
        }
        let mut left = match self.tokens.get(self.at) {
            Some(Token::Number(number)) => {
                self.at += 1;
                if evaluate { *number } else { 0 }
            }
            Some(Token::Op("(")) => {
                self.at += 1;
                let value = self.expression(0, evaluate, depth + 1)?;
                if !matches!(self.tokens.get(self.at), Some(Token::Op(")"))) {
                    return Err(fail("unclosed conditional parenthesis"));
                }
                self.at += 1;
                value
            }
            Some(Token::Op(operator @ ("!" | "~" | "+" | "-"))) => {
                self.at += 1;
                let value = self.expression(11, evaluate, depth + 1)?;
                if !evaluate {
                    0
                } else {
                    match *operator {
                        "!" => i32::from(value == 0),
                        "~" => !value,
                        "-" => value
                            .checked_neg()
                            .ok_or_else(|| fail("conditional integer overflow"))?,
                        _ => value,
                    }
                }
            }
            _ => return Err(fail("expected a conditional operand")),
        };
        while let Some(Token::Op(operator)) = self.tokens.get(self.at) {
            let Some(priority) = priority(operator) else {
                break;
            };
            if priority < precedence {
                break;
            }
            self.at += 1;
            let right_evaluated =
                evaluate && !((*operator == "&&" && left == 0) || (*operator == "||" && left != 0));
            let right = self.expression(priority + 1, right_evaluated, depth + 1)?;
            left = if evaluate {
                binary(operator, left, right)?
            } else {
                0
            };
        }
        Ok(left)
    }
}

fn priority(operator: &str) -> Option<u8> {
    Some(match operator {
        "||" => 0,
        "&&" => 1,
        "|" => 2,
        "^" => 3,
        "&" => 4,
        "==" | "!=" => 5,
        "<" | "<=" | ">" | ">=" => 6,
        "<<" | ">>" => 7,
        "+" | "-" => 8,
        "*" | "/" | "%" => 9,
        _ => return None,
    })
}

fn binary(operator: &str, left: i32, right: i32) -> Result<i32, Error> {
    let value = match operator {
        "+" => left.checked_add(right),
        "-" => left.checked_sub(right),
        "*" => left.checked_mul(right),
        "/" => left.checked_div(right),
        "%" => left.checked_rem(right),
        "<<" if (0..32).contains(&right) => i32::try_from(i64::from(left) << right).ok(),
        ">>" if (0..32).contains(&right) => Some(left >> right),
        "<<" | ">>" => None,
        "&" => Some(left & right),
        "|" => Some(left | right),
        "^" => Some(left ^ right),
        "&&" => Some(i32::from(left != 0 && right != 0)),
        "||" => Some(i32::from(left != 0 || right != 0)),
        "==" => Some(i32::from(left == right)),
        "!=" => Some(i32::from(left != right)),
        "<" => Some(i32::from(left < right)),
        "<=" => Some(i32::from(left <= right)),
        ">" => Some(i32::from(left > right)),
        ">=" => Some(i32::from(left >= right)),
        _ => None,
    };
    value.ok_or_else(|| fail("invalid or overflowing conditional arithmetic"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(source: &str) -> Result<bool, Error> {
        condition(source, &Macros::new(), &mut 65536)
    }

    #[test]
    fn operator_precedence_and_short_circuit_match_integer_conditions() {
        for source in [
            "1 + 2 * 3 == 7",
            "(1 + 2) * 3 == 9",
            "!0 && ~0 == -1",
            "0x10 >> 2 == 4",
            "1 << 3 == 8",
            "3 & 1 == 1",
            "7 % 4 == 3",
            "0 || 1 && 2",
            "1 || (1 / 0)",
            "!(0 && (1 / 0))",
        ] {
            assert!(value(source).unwrap(), "{source}");
        }
        assert!(!value("1 && 0 || 0").unwrap());
        assert!(!value("UNDEFINED").unwrap());
        for source in [
            "1 / 0",
            "2147483647 + 1",
            "1 << 32",
            "1 << -1",
            "0x80000000",
            "010",
            "1 ? 2 : 3",
            "1.5",
            "defined(1)",
            "1 2",
            "(",
        ] {
            assert!(value(source).is_err(), "{source}");
        }
    }

    #[test]
    fn macro_expansion_preserves_unparenthesized_replacement_precedence() {
        let macros = Macros::from([
            ("X".into(), Some(tokens("1 + 2").unwrap())),
            ("ALIAS".into(), Some(tokens("X").unwrap())),
            ("GUARD".into(), Some(vec![])),
            ("FUNCTION".into(), None),
        ]);
        assert!(
            condition(
                "ALIAS * 3 == 7 && defined(GUARD) && defined FUNCTION",
                &macros,
                &mut 65536
            )
            .unwrap()
        );
        assert!(condition("FUNCTION", &macros, &mut 65536).is_err());
        assert!(condition("GUARD", &macros, &mut 65536).is_err());
        assert!(value("defined(GL_ES)").is_err());
    }

    #[test]
    fn hostile_conditions_cannot_expand_or_recurse_without_bounds() {
        let recursive = Macros::from([("X".into(), Some(tokens("X").unwrap()))]);
        assert!(condition("X", &recursive, &mut 65536).is_err());
        assert!(value(&format!("{}1{}", "(".repeat(1000), ")".repeat(1000))).is_err());
        assert!(condition("1 + 2", &Macros::new(), &mut 2).is_err());
    }
}
