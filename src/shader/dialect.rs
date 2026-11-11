//! Wallpaper Engine's shaders are written to pass as HLSL too, which lets a vector start from a
//! number, a number from a vector, `%` work on fractions, integers stand for fractions, and a
//! `const` hold whatever it's given. GLSL takes none of that. This works out what type each
//! expression is where that's plain, and spells out the conversions HLSL would make, only
//! where GLSL would otherwise refuse the shader. Anything it can't follow is left as written.

use std::collections::{BTreeMap, HashMap};

/// The most edits one stage may get.
const MOST: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ty {
    Float,
    Int,
    Uint,
    Bool,
    Vec(u8),
    Mat(u8),
    Unknown,
}

impl Ty {
    fn named(name: &str) -> Option<Ty> {
        Some(match name {
            "float" | "half" => Ty::Float,
            "int" => Ty::Int,
            "uint" => Ty::Uint,
            "bool" => Ty::Bool,
            "vec2" | "float2" => Ty::Vec(2),
            "vec3" | "float3" => Ty::Vec(3),
            "vec4" | "float4" => Ty::Vec(4),
            "mat2" | "float2x2" => Ty::Mat(2),
            "mat3" | "float3x3" => Ty::Mat(3),
            "mat4" | "float4x4" => Ty::Mat(4),
            "ivec2"
            | "ivec3"
            | "ivec4"
            | "bvec2"
            | "bvec3"
            | "bvec4"
            | "uvec2"
            | "uvec3"
            | "uvec4"
            | "sampler2D"
            | "sampler2DComparison"
            | "samplerCube"
            | "void" => Ty::Unknown,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Name,
    Number { fraction: bool },
    Mark,
}

#[derive(Clone, Copy, Debug)]
struct Tok<'a> {
    text: &'a str,
    kind: Kind,
    start: usize,
    end: usize,
}

/// An edit to the source: text put in before or after a place, or put in place of a span.
#[derive(Debug)]
struct Edit {
    at: usize,
    end: usize,
    order: (u8, usize),
    text: String,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    start: usize,
    end: usize,
    ty: Ty,
    /// Made only of numbers and constants, so it may set a `const`.
    constant: bool,
    /// A whole number written as one.
    literal: bool,
}

#[derive(Clone, Copy)]
struct Var {
    ty: Ty,
    constant: bool,
    array: bool,
}

struct Function {
    returns: Ty,
    parameters: Vec<Ty>,
}

/// `source` with HLSL's conversions spelled out; `combos` are whole numbers it may use.
pub(super) fn converted(source: &str, combos: &BTreeMap<String, i32>) -> String {
    let tokens = lex(source);
    let constant = |ty| Var {
        ty,
        constant: true,
        array: false,
    };
    let mut globals: HashMap<&str, Var> = combos
        .keys()
        .map(|name| (name.as_str(), constant(Ty::Int)))
        .collect();
    // Macros that stand for a number are that number's type.
    for line in source.lines() {
        let mut words = line
            .trim_start()
            .strip_prefix("#define")
            .unwrap_or("")
            .split_whitespace();
        if let (Some(name), Some(value), None) = (words.next(), words.next(), words.next()) {
            let ty = if value.parse::<i64>().is_ok() {
                Ty::Int
            } else if value.parse::<f64>().is_ok() {
                Ty::Float
            } else {
                continue;
            };
            globals.insert(name, constant(ty));
        }
    }
    let mut pass = Pass {
        tokens: &tokens,
        at: 0,
        edits: Vec::new(),
        sequence: 0,
        globals,
        functions: HashMap::new(),
        scopes: Vec::new(),
        returns: Ty::Unknown,
        broken: false,
    };
    pass.declare_all();
    pass.at = 0;
    pass.bodies();
    if pass.broken || pass.edits.is_empty() || pass.edits.len() > MOST {
        return source.to_owned();
    }
    let mut edits = pass.edits;
    edits.sort_by_key(|edit| (edit.at, edit.order));
    let mut out = String::with_capacity(source.len() + edits.len() * 8);
    let mut from = 0;
    for edit in edits {
        if edit.at < from {
            return source.to_owned();
        }
        out.push_str(&source[from..edit.at]);
        out.push_str(&edit.text);
        from = edit.end;
    }
    out.push_str(&source[from..]);
    out
}

fn lex(source: &str) -> Vec<Tok<'_>> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut at = 0;
    let mut line_start = true;
    while at < bytes.len() {
        let c = bytes[at];
        if c == b'\n' {
            line_start = true;
            at += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            at += 1;
            continue;
        }
        if bytes[at..].starts_with(b"//") {
            at += source[at..].find('\n').unwrap_or(bytes.len() - at);
            continue;
        }
        if bytes[at..].starts_with(b"/*") {
            at += source[at..]
                .find("*/")
                .map_or(bytes.len() - at, |end| end + 2);
            continue;
        }
        // A directive line says nothing about types here.
        if c == b'#' && line_start {
            at += source[at..].find('\n').unwrap_or(bytes.len() - at);
            continue;
        }
        line_start = false;
        let start = at;
        let kind = if c.is_ascii_alphabetic() || c == b'_' {
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                at += 1;
            }
            Kind::Name
        } else if c == b'0' && matches!(bytes.get(at + 1), Some(b'x' | b'X')) {
            // Hex digits include `f`, which mustn't read as a fraction's suffix.
            at += 2;
            while at < bytes.len() && bytes[at].is_ascii_hexdigit() {
                at += 1;
            }
            if at < bytes.len() && matches!(bytes[at], b'u' | b'U') {
                at += 1;
            }
            Kind::Number { fraction: false }
        } else if c.is_ascii_digit()
            || (c == b'.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        {
            let mut fraction = false;
            while at < bytes.len() {
                let d = bytes[at];
                if d.is_ascii_digit() {
                    at += 1;
                } else if d == b'.' {
                    fraction = true;
                    at += 1;
                } else if matches!(d, b'e' | b'E')
                    && bytes
                        .get(at + 1)
                        .is_some_and(|n| n.is_ascii_digit() || matches!(n, b'-' | b'+'))
                {
                    fraction = true;
                    at += 2;
                } else if matches!(d, b'f' | b'F') {
                    fraction = true;
                    at += 1;
                    break;
                } else if matches!(d, b'u' | b'U' | b'x' | b'X')
                    || (d.is_ascii_hexdigit() && !d.is_ascii_digit())
                {
                    at += 1;
                } else {
                    break;
                }
            }
            Kind::Number { fraction }
        } else {
            let two = source.get(at..at + 2).unwrap_or("");
            at += if matches!(
                two,
                "==" | "!="
                    | "<="
                    | ">="
                    | "&&"
                    | "||"
                    | "^^"
                    | "++"
                    | "--"
                    | "+="
                    | "-="
                    | "*="
                    | "/="
                    | "%="
                    | "<<"
                    | ">>"
                    | "&="
                    | "|="
                    | "^="
            ) {
                2
            } else {
                source[at..].chars().next().map_or(1, char::len_utf8)
            };
            Kind::Mark
        };
        tokens.push(Tok {
            text: &source[start..at],
            kind,
            start,
            end: at,
        });
    }
    tokens
}

const QUALIFIERS: &[&str] = &[
    "uniform",
    "varying",
    "attribute",
    "in",
    "out",
    "inout",
    "const",
    "highp",
    "mediump",
    "lowp",
    "flat",
    "smooth",
    "centroid",
    "static",
    "invariant",
    "precise",
];

const PER_COMPONENT: &[&str] = &[
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "pow",
    "exp",
    "log",
    "exp2",
    "log2",
    "sqrt",
    "inversesqrt",
    "rsqrt",
    "abs",
    "sign",
    "floor",
    "ceil",
    "fract",
    "frac",
    "mod",
    "fmod",
    "min",
    "max",
    "clamp",
    "saturate",
    "mix",
    "lerp",
    "step",
    "smoothstep",
    "normalize",
    "reflect",
    "radians",
    "degrees",
    "trunc",
    "round",
    "dFdx",
    "dFdy",
    "ddx",
    "ddy",
    "fwidth",
    "length",
    "distance",
    "dot",
    "cross",
];

struct Pass<'a, 'b> {
    tokens: &'b [Tok<'a>],
    at: usize,
    edits: Vec<Edit>,
    sequence: usize,
    globals: HashMap<&'a str, Var>,
    functions: HashMap<&'a str, Option<Function>>,
    scopes: Vec<HashMap<&'a str, Var>>,
    returns: Ty,
    broken: bool,
}

impl<'a> Pass<'a, '_> {
    fn peek(&self, ahead: usize) -> Option<Tok<'a>> {
        self.tokens.get(self.at + ahead).copied()
    }

    fn text(&self, ahead: usize) -> &'a str {
        self.peek(ahead).map_or("", |token| token.text)
    }

    fn eat(&mut self, text: &str) -> bool {
        if self.text(0) == text {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, text: &str) {
        if !self.eat(text) {
            self.broken = true;
        }
    }

    fn done(&self) -> bool {
        self.broken || self.at >= self.tokens.len()
    }

    /// Past a bracketed run starting at an opening bracket.
    fn skip_group(&mut self) {
        let mut depth = 0usize;
        while let Some(token) = self.peek(0) {
            self.at += 1;
            match token.text {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_statement(&mut self) {
        while let Some(token) = self.peek(0) {
            match token.text {
                ";" => {
                    self.at += 1;
                    return;
                }
                "(" | "[" | "{" => self.skip_group(),
                _ => self.at += 1,
            }
        }
    }

    // Edits.

    fn edit(&mut self, at: usize, end: usize, side: u8, text: impl Into<String>) {
        self.sequence += 1;
        // Closings first, then replacements, then openings, the outermost opening first.
        let order = match side {
            0 => (0, self.sequence),
            1 => (1, self.sequence),
            _ => (2, usize::MAX - self.sequence),
        };
        self.edits.push(Edit {
            at,
            end,
            order,
            text: text.into(),
        });
    }

    fn wrap(&mut self, node: &Node, open: &str, close: &str) {
        self.edit(node.end, node.end, 0, close);
        self.edit(node.start, node.start, 2, open);
    }

    /// `node` made `to`, as HLSL would make it. Returns the type it ends up.
    fn convert(&mut self, node: &Node, to: Ty) -> Ty {
        match (node.ty, to) {
            (from, to) if from == to || from == Ty::Unknown || to == Ty::Unknown => node.ty,
            (Ty::Int, Ty::Float) if node.literal => {
                self.edit(node.end, node.end, 0, ".0");
                to
            }
            (Ty::Int, Ty::Uint) if node.literal => {
                self.edit(node.end, node.end, 0, "u");
                to
            }
            (Ty::Int | Ty::Uint | Ty::Bool, Ty::Float) => {
                self.wrap(node, "float(", ")");
                to
            }
            (Ty::Float | Ty::Uint | Ty::Bool, Ty::Int) => {
                self.wrap(node, "int(", ")");
                to
            }
            (Ty::Float | Ty::Int | Ty::Bool, Ty::Uint) => {
                self.wrap(node, "uint(", ")");
                to
            }
            (Ty::Float | Ty::Int | Ty::Uint, Ty::Vec(n)) => {
                self.wrap(node, &format!("vec{n}("), ")");
                to
            }
            (Ty::Vec(_), Ty::Float) => {
                self.wrap(node, "(", ").x");
                to
            }
            (Ty::Vec(m), Ty::Vec(n)) if m > n => {
                self.wrap(node, "(", &format!(").{}", &"xyzw"[..n as usize]));
                to
            }
            _ => node.ty,
        }
    }

    // Declarations.

    fn type_at(&self, ahead: usize) -> Option<(Ty, usize)> {
        let mut skip = ahead;
        while QUALIFIERS.contains(&self.text(skip)) {
            skip += 1;
        }
        let ty = Ty::named(self.text(skip))?;
        let name = self.peek(skip + 1)?;
        (name.kind == Kind::Name && !QUALIFIERS.contains(&name.text)).then_some((ty, skip))
    }

    /// Every global and function, so bodies can call what comes later.
    fn declare_all(&mut self) {
        while !self.done() {
            match self.text(0) {
                "struct" => {
                    while !self.done() && self.text(0) != "{" {
                        self.at += 1;
                    }
                    self.skip_group();
                    self.skip_statement();
                    continue;
                }
                "precision" => {
                    self.skip_statement();
                    continue;
                }
                _ => {}
            }
            let Some((ty, skip)) = self.type_at(0) else {
                if self.text(0) == "{" || self.text(0) == "(" {
                    self.skip_group();
                } else {
                    self.at += 1;
                }
                continue;
            };
            let constant = (0..skip).any(|ahead| self.text(ahead) == "const");
            self.at += skip + 1;
            let name = self.text(0);
            self.at += 1;
            if self.text(0) == "(" {
                let parameters = self.parameters();
                let known = Function {
                    returns: ty,
                    parameters: parameters.into_iter().map(|(ty, _)| ty).collect(),
                };
                // Overloads aren't told apart.
                let entry = self.functions.entry(name).or_insert(Some(known));
                if let Some(existing) = entry
                    && existing.returns != ty
                {
                    *entry = None;
                }
                if self.text(0) == "{" {
                    self.skip_group();
                } else {
                    self.skip_statement();
                }
                continue;
            }
            self.globals.insert(
                name,
                Var {
                    ty,
                    constant,
                    array: self.text(0) == "[",
                },
            );
            // Further names in one declaration.
            while !self.done() && self.text(0) != ";" {
                if self.text(0) == "," && self.peek(1).is_some_and(|t| t.kind == Kind::Name) {
                    let next = self.text(1);
                    self.at += 2;
                    let array = self.text(0) == "[";
                    self.globals.insert(
                        next,
                        Var {
                            ty,
                            constant,
                            array,
                        },
                    );
                    continue;
                }
                if matches!(self.text(0), "(" | "[" | "{") {
                    self.skip_group();
                } else {
                    self.at += 1;
                }
            }
            self.at += 1;
        }
    }

    /// A parameter list from its `(`, past its `)`.
    fn parameters(&mut self) -> Vec<(Ty, &'a str)> {
        let mut out = Vec::new();
        self.at += 1;
        while !self.done() && self.text(0) != ")" {
            if let Some((ty, skip)) = self.type_at(0) {
                self.at += skip + 1;
                out.push((ty, self.text(0)));
                self.at += 1;
            } else if self.text(0) == "void" {
                self.at += 1;
                continue;
            } else {
                out.push((Ty::Unknown, ""));
            }
            while !self.done() && !matches!(self.text(0), "," | ")") {
                if self.text(0) == "[" {
                    self.skip_group();
                } else {
                    self.at += 1;
                }
            }
            self.eat(",");
        }
        self.at += 1;
        out
    }

    fn bodies(&mut self) {
        while !self.done() {
            match self.text(0) {
                "struct" => {
                    while !self.done() && self.text(0) != "{" {
                        self.at += 1;
                    }
                    self.skip_group();
                    self.skip_statement();
                    continue;
                }
                "{" | "(" => {
                    self.skip_group();
                    continue;
                }
                _ => {}
            }
            let Some((ty, skip)) = self.type_at(0) else {
                self.at += 1;
                continue;
            };
            if self.text(skip + 2) != "(" {
                self.skip_statement();
                continue;
            }
            self.at += skip + 2;
            let parameters = self.parameters();
            if self.text(0) != "{" {
                self.skip_statement();
                continue;
            }
            self.returns = ty;
            let scope = parameters
                .into_iter()
                .filter(|(_, name)| !name.is_empty())
                .map(|(ty, name)| {
                    (
                        name,
                        Var {
                            ty,
                            constant: false,
                            array: false,
                        },
                    )
                })
                .collect();
            self.scopes = vec![scope];
            self.statement();
            self.scopes.clear();
        }
    }

    fn lookup(&self, name: &str) -> Option<Var> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
            .or_else(|| self.globals.get(name).copied())
    }

    // Statements.

    fn statement(&mut self) {
        if self.done() {
            return;
        }
        match self.text(0) {
            "{" => {
                self.at += 1;
                self.scopes.push(HashMap::new());
                while !self.done() && self.text(0) != "}" {
                    self.statement();
                }
                self.scopes.pop();
                self.expect("}");
            }
            "if" | "while" => {
                self.at += 1;
                self.expect("(");
                self.expression();
                self.expect(")");
                self.statement();
                if self.eat("else") {
                    self.statement();
                }
            }
            "do" => {
                self.at += 1;
                self.statement();
                self.expect("while");
                self.expect("(");
                self.expression();
                self.expect(")");
                self.expect(";");
            }
            "for" => {
                self.at += 1;
                self.expect("(");
                self.scopes.push(HashMap::new());
                if self.type_at(0).is_some() {
                    self.declaration();
                } else {
                    if self.text(0) != ";" {
                        self.expression();
                    }
                    self.expect(";");
                }
                if self.text(0) != ";" {
                    self.expression();
                }
                self.expect(";");
                if self.text(0) != ")" {
                    self.expression();
                }
                self.expect(")");
                self.statement();
                self.scopes.pop();
            }
            "return" => {
                self.at += 1;
                if self.text(0) != ";" {
                    let node = self.expression();
                    let returns = self.returns;
                    self.convert(&node, returns);
                }
                self.expect(";");
            }
            "break" | "continue" | "discard" | ";" => self.skip_statement(),
            _ if self.type_at(0).is_some() => self.declaration(),
            _ => {
                self.expression();
                if !self.eat(";") {
                    self.skip_statement();
                }
            }
        }
    }

    /// A local declaration, through its `;`.
    fn declaration(&mut self) {
        let Some((ty, skip)) = self.type_at(0) else {
            self.skip_statement();
            return;
        };
        let qualifier = (0..skip)
            .map(|ahead| self.peek(ahead))
            .find(|token| token.is_some_and(|token| token.text == "const"))
            .flatten();
        self.at += skip + 1;
        let mut all_constant = true;
        loop {
            let name = self.text(0);
            self.at += 1;
            let array = self.text(0) == "[";
            if array {
                self.skip_group();
            }
            let mut constant = qualifier.is_some();
            if self.eat("=") {
                let node = self.assignment();
                if !array {
                    self.convert(&node, ty);
                }
                constant &= node.constant;
                all_constant &= node.constant;
            }
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(
                    name,
                    Var {
                        ty,
                        constant,
                        array,
                    },
                );
            }
            if !self.eat(",") {
                break;
            }
        }
        // HLSL's `const` holds anything; GLSL's only what's known before it runs.
        if let Some(qualifier) = qualifier
            && !all_constant
        {
            self.edit(qualifier.start, qualifier.end, 1, "");
        }
        if !self.eat(";") {
            self.skip_statement();
        }
    }

    // Expressions, from the loosest binding in.

    fn expression(&mut self) -> Node {
        let mut node = self.assignment();
        while self.text(0) == "," {
            self.at += 1;
            let next = self.assignment();
            node = Node {
                end: next.end,
                ..next
            };
        }
        node
    }

    fn assignment(&mut self) -> Node {
        let target = self.conditional();
        let operator = self.text(0);
        if !matches!(operator, "=" | "+=" | "-=" | "*=" | "/=" | "%=") {
            return target;
        }
        self.at += 1;
        let value = self.assignment();
        match operator {
            "=" => {
                self.convert(&value, target.ty);
            }
            "%=" if target.ty != Ty::Int && target.ty != Ty::Unknown => {
                // a %= b as a = mod(a, b) needs the target written twice; left as it is.
            }
            _ if matches!(value.ty, Ty::Int | Ty::Uint | Ty::Bool)
                && matches!(target.ty, Ty::Float | Ty::Vec(_)) =>
            {
                self.convert(&value, Ty::Float);
            }
            _ if matches!(target.ty, Ty::Int | Ty::Uint) && value.ty != Ty::Float => {
                self.convert(&value, target.ty);
            }
            _ => {}
        }
        Node {
            start: target.start,
            end: value.end,
            constant: false,
            literal: false,
            ty: target.ty,
        }
    }

    fn conditional(&mut self) -> Node {
        let test = self.binary(0);
        if !self.eat("?") {
            return test;
        }
        let yes = self.assignment();
        self.expect(":");
        let no = self.assignment();
        let ty = match (yes.ty, no.ty) {
            (Ty::Int, Ty::Float) => self.convert(&yes, Ty::Float),
            (Ty::Float, Ty::Int) => self.convert(&no, Ty::Float),
            (a, b) if a == b => a,
            _ => Ty::Unknown,
        };
        Node {
            start: test.start,
            end: no.end,
            ty,
            constant: test.constant && yes.constant && no.constant,
            literal: false,
        }
    }

    fn binary(&mut self, floor: u8) -> Node {
        let mut left = self.unary();
        loop {
            let operator = self.text(0);
            let level = match operator {
                "||" => 1,
                "^^" => 2,
                "&&" => 3,
                "|" => 4,
                "^" => 5,
                "&" => 6,
                "==" | "!=" => 7,
                "<" | ">" | "<=" | ">=" => 8,
                "<<" | ">>" => 9,
                "+" | "-" => 10,
                "*" | "/" | "%" => 11,
                _ => return left,
            };
            if level <= floor {
                return left;
            }
            let mark = self.peek(0);
            self.at += 1;
            let right = self.binary(level);
            left = self.combine(left, operator, mark, right);
        }
    }

    fn combine(&mut self, left: Node, operator: &str, mark: Option<Tok>, right: Node) -> Node {
        let constant = left.constant && right.constant;
        let span = |ty| Node {
            start: left.start,
            end: right.end,
            ty,
            constant,
            literal: false,
        };
        let numeric =
            |ty: Ty| matches!(ty, Ty::Float | Ty::Int | Ty::Uint | Ty::Vec(_) | Ty::Mat(_));
        match operator {
            "||" | "^^" | "&&" => span(Ty::Bool),
            "|" | "^" | "&" | "<<" | ">>" => span(Ty::Unknown),
            "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                self.promote(&left, &right);
                span(Ty::Bool)
            }
            _ => {
                let (a, b) = self.promote(&left, &right);
                if !numeric(a) || !numeric(b) {
                    return span(Ty::Unknown);
                }
                let ty = match (a, b) {
                    (Ty::Mat(_), Ty::Vec(n)) | (Ty::Vec(n), Ty::Mat(_)) => Ty::Vec(n),
                    (Ty::Mat(n), Ty::Mat(_)) | (Ty::Mat(n), _) | (_, Ty::Mat(n)) => Ty::Mat(n),
                    (Ty::Vec(n), Ty::Vec(m)) if n == m => Ty::Vec(n),
                    (Ty::Vec(_), Ty::Vec(_)) => Ty::Unknown,
                    (Ty::Vec(n), _) | (_, Ty::Vec(n)) => Ty::Vec(n),
                    (Ty::Float, _) | (_, Ty::Float) => Ty::Float,
                    (Ty::Uint, Ty::Uint) => Ty::Uint,
                    (Ty::Int, Ty::Int) => Ty::Int,
                    _ => Ty::Unknown,
                };
                if operator == "%"
                    && matches!(ty, Ty::Float | Ty::Vec(_))
                    && let Some(mark) = mark
                {
                    self.edit(left.start, left.start, 2, "mod(");
                    self.edit(mark.start, mark.end, 1, ",");
                    self.edit(right.end, right.end, 0, ")");
                }
                span(ty)
            }
        }
    }

    /// An integer beside a fraction or a vector made a fraction, as HLSL makes it.
    fn promote(&mut self, left: &Node, right: &Node) -> (Ty, Ty) {
        let wide = |ty| matches!(ty, Ty::Float | Ty::Vec(_) | Ty::Mat(_));
        let whole = |ty| matches!(ty, Ty::Int | Ty::Uint | Ty::Bool);
        match (left.ty, right.ty) {
            (a, other) if whole(a) && wide(other) => (self.convert(left, Ty::Float), other),
            (other, b) if whole(b) && wide(other) => (other, self.convert(right, Ty::Float)),
            (Ty::Uint, Ty::Int | Ty::Bool) => (Ty::Uint, self.convert(right, Ty::Uint)),
            (Ty::Int | Ty::Bool, Ty::Uint) => (self.convert(left, Ty::Uint), Ty::Uint),
            (Ty::Bool, Ty::Int) => (self.convert(left, Ty::Int), Ty::Int),
            (Ty::Int, Ty::Bool) => (Ty::Int, self.convert(right, Ty::Int)),
            pair => pair,
        }
    }

    fn unary(&mut self) -> Node {
        let Some(token) = self.peek(0) else {
            self.broken = true;
            return Node {
                start: 0,
                end: 0,
                ty: Ty::Unknown,
                constant: false,
                literal: false,
            };
        };
        if matches!(token.text, "-" | "+" | "!" | "~" | "++" | "--") {
            self.at += 1;
            let inner = self.unary();
            let ty = if token.text == "!" {
                Ty::Bool
            } else {
                inner.ty
            };
            return Node {
                start: token.start,
                end: inner.end,
                ty,
                constant: inner.constant,
                literal: inner.literal && token.text == "-",
            };
        }
        let mut node = self.primary();
        loop {
            match self.text(0) {
                "." => {
                    self.at += 1;
                    let Some(member) = self.peek(0) else {
                        self.broken = true;
                        return node;
                    };
                    self.at += 1;
                    let swizzle = member.text.len() <= 4
                        && (member.text.chars().all(|c| "xyzw".contains(c))
                            || member.text.chars().all(|c| "rgba".contains(c))
                            || member.text.chars().all(|c| "stpq".contains(c)));
                    let ty = match node.ty {
                        Ty::Vec(_) if swizzle && member.text.len() == 1 => Ty::Float,
                        Ty::Vec(_) if swizzle => Ty::Vec(member.text.len() as u8),
                        _ => Ty::Unknown,
                    };
                    node = Node {
                        end: member.end,
                        ty,
                        literal: false,
                        ..node
                    };
                }
                "[" => {
                    self.at += 1;
                    let index = self.expression();
                    if index.ty == Ty::Float {
                        self.convert(&index, Ty::Int);
                    }
                    let close = self.peek(0);
                    self.expect("]");
                    let ty = match node.ty {
                        Ty::Vec(_) => Ty::Float,
                        Ty::Mat(n) => Ty::Vec(n),
                        _ => Ty::Unknown,
                    };
                    node = Node {
                        end: close.map_or(node.end, |close| close.end),
                        ty,
                        constant: false,
                        literal: false,
                        ..node
                    };
                }
                "++" | "--" => {
                    let end = self.peek(0).map_or(node.end, |token| token.end);
                    self.at += 1;
                    node = Node {
                        end,
                        constant: false,
                        literal: false,
                        ..node
                    };
                }
                _ => return node,
            }
        }
    }

    fn primary(&mut self) -> Node {
        let Some(token) = self.peek(0) else {
            self.broken = true;
            return Node {
                start: 0,
                end: 0,
                ty: Ty::Unknown,
                constant: false,
                literal: false,
            };
        };
        self.at += 1;
        let plain = |ty, constant| Node {
            start: token.start,
            end: token.end,
            ty,
            constant,
            literal: false,
        };
        match token.kind {
            Kind::Number { fraction } => {
                let ty = if fraction {
                    Ty::Float
                } else if token.text.ends_with(['u', 'U']) {
                    Ty::Uint
                } else {
                    Ty::Int
                };
                Node {
                    literal: ty == Ty::Int,
                    ..plain(ty, true)
                }
            }
            Kind::Mark if token.text == "(" => {
                let inner = self.expression();
                let close = self.peek(0);
                self.expect(")");
                Node {
                    start: token.start,
                    end: close.map_or(inner.end, |close| close.end),
                    literal: false,
                    ..inner
                }
            }
            Kind::Mark => {
                self.broken = true;
                plain(Ty::Unknown, false)
            }
            Kind::Name if self.text(0) == "(" => self.call(token),
            Kind::Name => match token.text {
                "true" | "false" => plain(Ty::Bool, true),
                name => match self.lookup(name) {
                    Some(var) if !var.array => plain(var.ty, var.constant),
                    _ => plain(Ty::Unknown, false),
                },
            },
        }
    }

    fn call(&mut self, name: Tok<'a>) -> Node {
        self.at += 1;
        let mut arguments = Vec::new();
        while !self.done() && self.text(0) != ")" {
            arguments.push(self.assignment());
            if !self.eat(",") {
                break;
            }
        }
        let close = self.peek(0);
        self.expect(")");
        let end = close.map_or(name.end, |close| close.end);
        let all_constant = arguments.iter().all(|argument| argument.constant);
        let node = |ty, constant| Node {
            start: name.start,
            end,
            ty,
            constant,
            literal: false,
        };
        if let Some(ty) = Ty::named(name.text) {
            return node(ty, all_constant);
        }
        if let Some(Some(function)) = self.functions.get(name.text) {
            let (parameters, returns) = (function.parameters.clone(), function.returns);
            if parameters.len() == arguments.len() {
                for (argument, parameter) in arguments.iter().zip(parameters) {
                    self.convert(argument, parameter);
                }
            }
            return node(returns, false);
        }
        let types: Vec<Ty> = arguments.iter().map(|argument| argument.ty).collect();
        let widest = types
            .iter()
            .copied()
            .find(|ty| matches!(ty, Ty::Vec(_)))
            .or_else(|| types.iter().copied().find(|ty| *ty == Ty::Float))
            .unwrap_or(Ty::Float);
        let ty = match name.text {
            "texSample2D" | "texSample2DLod" | "texture" | "texture2D" | "textureLod" => Ty::Vec(4),
            "CAST2" => Ty::Vec(2),
            "CAST3" => Ty::Vec(3),
            "CAST4" => Ty::Vec(4),
            "mul" => match (types.first(), types.get(1)) {
                (Some(Ty::Vec(n)), Some(Ty::Mat(_))) | (Some(Ty::Mat(_)), Some(Ty::Vec(n))) => {
                    Ty::Vec(*n)
                }
                _ => Ty::Unknown,
            },
            builtin if PER_COMPONENT.contains(&builtin) => {
                if types
                    .iter()
                    .any(|ty| matches!(ty, Ty::Unknown | Ty::Bool | Ty::Mat(_)))
                {
                    return node(Ty::Unknown, false);
                }
                for argument in &arguments {
                    if argument.ty == Ty::Int {
                        self.convert(argument, Ty::Float);
                    }
                }
                // Where GLSL wants the same type in two places, the number goes wide.
                let paired = match builtin {
                    "pow" | "atan" | "atan2" | "fmod" | "distance" | "dot" | "reflect" => 2,
                    "mix" | "lerp" => 2,
                    _ => 0,
                };
                if let Ty::Vec(_) = widest {
                    for argument in arguments.iter().take(paired) {
                        if argument.ty != widest {
                            self.convert(argument, widest);
                        }
                    }
                }
                match builtin {
                    "length" | "distance" | "dot" => Ty::Float,
                    "cross" => Ty::Vec(3),
                    "step" | "smoothstep" => types
                        .last()
                        .copied()
                        .map_or(Ty::Unknown, |ty| if ty == Ty::Int { Ty::Float } else { ty }),
                    _ => widest,
                }
            }
            _ => Ty::Unknown,
        };
        node(
            ty,
            all_constant && ty != Ty::Unknown && !name.text.starts_with("tex"),
        )
    }
}

#[cfg(test)]
mod tests;
