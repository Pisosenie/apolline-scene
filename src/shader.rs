//! Preparation of the supported fragment-shader subset. No source is executed here.

mod conditional;
mod expression;
mod parameters;
mod samplers;

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path};
use std::sync::Arc;

use crate::Error;
pub(crate) use parameters::Constant;
pub use parameters::Reads;
use parameters::Stage;
use serde_json::{Map, Value};

const FILE_BYTES: usize = 256 * 1024;
const TOTAL_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Fragment {
    pub(crate) source: String,
    pub(crate) texcoord: u8,
    pub(crate) inputs: u8,
    pub(crate) animated: bool,
    /// Material values, set as uniforms for each layer; layers that differ only in these
    /// share a program.
    pub(crate) constants: Vec<(String, Constant)>,
    /// The material value each constant was given from, by the material's name for it; none
    /// for a shader's default.
    pub(crate) keys: Vec<Option<String>>,
    /// The material's own vertex shader, when it has one.
    pub(crate) vertex: Option<String>,
    /// What it reads from the engine beyond time.
    pub reads: Reads,
    /// Its vertex shader takes `a_Position` as already where it goes on the target, as
    /// Wallpaper Engine draws passes into an effect's buffers, rather than placing it through
    /// the matrix.
    pub(crate) unplaced: bool,
    bindings: Option<u8>,
    pub(crate) files: Vec<String>,
}

impl Fragment {
    pub(crate) fn validate_inputs(&self, bindings: u8) -> Result<(), Error> {
        if bindings & 1 == 0 || self.inputs & !bindings != 0 {
            return Err(fail("material does not bind all required texture slots"));
        }
        if self.bindings.is_some_and(|prepared| prepared != bindings) {
            return Err(fail("texture bindings changed after fragment preparation"));
        }
        Ok(())
    }

    /// Prepares one fragment pass; time uniforms are supplied by the host. The loader only returns resources from its host.
    pub fn prepare(
        entry: &str,
        combos: &Map<String, Value>,
        constants: &Map<String, Value>,
        load: impl FnMut(&str) -> Result<Option<String>, Error>,
    ) -> Result<Self, Error> {
        Self::prepare_inner(entry, combos, constants, None, load)
    }

    /// Prepares texture-driven combo options for an explicit mask of bound sampler slots.
    pub fn prepare_bound(
        entry: &str,
        combos: &Map<String, Value>,
        constants: &Map<String, Value>,
        bindings: u8,
        load: impl FnMut(&str) -> Result<Option<String>, Error>,
    ) -> Result<Self, Error> {
        if bindings & 1 == 0 {
            return Err(fail("image materials require texture slot zero"));
        }
        Self::prepare_inner(entry, combos, constants, Some(bindings), load)
    }

    fn prepare_inner(
        entry: &str,
        combos: &Map<String, Value>,
        constants: &Map<String, Value>,
        bindings: Option<u8>,
        load: impl FnMut(&str) -> Result<Option<String>, Error>,
    ) -> Result<Self, Error> {
        Template::load(entry, load)?.prepare(combos, constants, bindings, None)
    }
}

pub(crate) struct Template {
    output: String,
    combos: BTreeMap<String, i32>,
    files: Vec<String>,
}

impl Template {
    pub(crate) fn load(
        entry: &str,
        load: impl FnMut(&str) -> Result<Option<String>, Error>,
    ) -> Result<Self, Error> {
        let entry = path("shaders", entry)?;
        let mut expansion = Expansion {
            load,
            cache: HashMap::new(),
            files: Vec::new(),
            stack: Vec::new(),
            inputs: 0,
            includes: 0,
            output: String::new(),
            combos: BTreeMap::new(),
        };
        expansion.expand(&entry)?;
        Ok(Self {
            output: expansion.output,
            combos: expansion.combos,
            files: expansion.files,
        })
    }

    /// Fills texture slots the material leaves empty from the shader's own defaults, read
    /// from the source as the material's combos select it, so a default in a branch that
    /// isn't taken neither loads nor refuses anything.
    pub(crate) fn fill_defaults(
        &self,
        names: &mut Vec<Option<String>>,
        combos: &Map<String, Value>,
    ) -> Result<(), Error> {
        let mut values = self.combos.clone();
        for (name, value) in combos {
            if values.contains_key(name) {
                values.insert(name.clone(), integer(value)?);
            }
        }
        let selected = conditional::select(&self.output, &values)?;
        samplers::fill_defaults(&selected, names)
    }

    pub(crate) fn prepare(
        self,
        combos: &Map<String, Value>,
        constants: &Map<String, Value>,
        bindings: Option<u8>,
        vertex: Option<Template>,
    ) -> Result<Fragment, Error> {
        if bindings.is_some_and(|mask| mask & 1 == 0) {
            return Err(fail("image materials require texture slot zero"));
        }
        if combos.len() > 64
            || combos
                .keys()
                .any(|name| !identifier(name) || protected(name))
        {
            return Err(fail("invalid or excessive material combo names"));
        }
        let mut expansion = self;
        let optional = if let Some(bindings) = bindings {
            Some(samplers::resolve(
                &expansion.output,
                bindings,
                combos,
                &mut expansion.combos,
            )?)
        } else {
            None
        };
        let controlled = optional
            .as_ref()
            .map(|options| options.controlled.clone())
            .unwrap_or_default();
        // A combo the material sets is defined for both stages, declared or not, as the engine
        // does; the vertex shader's own decide the fragment's branches too.
        for (name, value) in combos {
            if !controlled.contains(name) {
                expansion.combos.insert(name.clone(), integer(value)?);
            }
        }
        if let Some(vertex) = &vertex {
            for (name, default) in &vertex.combos {
                expansion.combos.entry(name.clone()).or_insert(*default);
            }
        }
        let missing = optional.as_ref().map(|options| options.missing);
        let selected = stage(&expansion.output, &expansion.combos, &controlled)?;
        let mut bound = parameters::bind(&selected, constants, missing, Stage::Fragment)?;
        let mut files = expansion.files;
        let fragment_tokens = parameters::tokens(&selected)?;
        // A varying the fragment declares but never reads needn't be written.
        let read = |name: &str| {
            fragment_tokens
                .iter()
                .filter(|token| token.text == name)
                .count()
                > 1
        };
        // A constant no uniform in the code that runs takes is let be, as the engine does,
        // unless one names it and wasn't read.
        let mut named: Vec<&String> = constants
            .keys()
            .filter(|key| annotated(&selected, key))
            .collect();
        let mut unplaced = false;
        let vertex = match vertex {
            Some(vertex) => {
                // Both stages see the same combos: the fragment's, as the material and its
                // textures set them, and the vertex shader's own beside them.
                let mut values = expansion.combos.clone();
                for (name, default) in &vertex.combos {
                    if !values.contains_key(name) {
                        let chosen = combos.get(name).map(integer).transpose()?;
                        values.insert(name.clone(), chosen.unwrap_or(*default));
                    }
                }
                let selected = stage(&vertex.output, &values, &controlled)?;
                named.extend(constants.keys().filter(|key| annotated(&selected, key)));
                let mut stage = parameters::bind(&selected, constants, missing, Stage::Vertex)?;
                let tokens = parameters::tokens(&selected)?;
                let named = |name: &str| tokens.iter().any(|token| token.text == name);
                // The matrix counts when it's used, not just declared.
                let used =
                    |name: &str| tokens.iter().filter(|token| token.text == name).count() > 1;
                unplaced = named("a_Position") && !used("g_ModelViewProjectionMatrix");
                for varying in &bound.varyings {
                    let written = stage.varyings.iter().find(|(name, _)| *name == varying.0);
                    if written.is_some_and(|written| written.1 != varying.1)
                        || (written.is_none() && read(&varying.0))
                    {
                        return Err(fail(format!(
                            "varying {} isn't written by the vertex shader",
                            varying.0
                        )));
                    }
                }
                // Each stage of a stock shader has values of its own, even under one name;
                // where they differ, in value or in the material value that feeds them, the
                // vertex stage's gets a name of its own.
                let keys = std::mem::take(&mut stage.keys);
                for ((name, value), key) in
                    std::mem::take(&mut stage.constants).into_iter().zip(keys)
                {
                    let fragment = bound.constants.iter().position(|(known, _)| *known == name);
                    match fragment.map(|at| (&bound.constants[at].1, &bound.keys[at])) {
                        Some((known, known_key)) if *known != value || *known_key != key => {
                            let own = format!("apolline_vertex_{name}");
                            stage.body = rename(&stage.body, &name, &own)?;
                            bound.constants.push((own, value));
                            bound.keys.push(key);
                        }
                        Some(_) => {}
                        None => {
                            bound.constants.push((name, value));
                            bound.keys.push(key);
                        }
                    }
                }
                bound.used.extend(stage.used);
                bound.inputs |= stage.inputs;
                bound.animated |= stage.animated;
                bound.reads = bound.reads.with(stage.reads);
                files.extend(vertex.files);
                Some(source(VERTEX, &values, &stage.body, VERTEX_MAIN)?)
            }
            None => {
                // Ours fills one plain v_TexCoord, not an array of them.
                if let Some((name, _)) = bound
                    .varyings
                    .iter()
                    .find(|(name, kind)| name != "v_TexCoord" || kind.contains('['))
                {
                    return Err(fail(format!("unsupported varying {name}")));
                }
                None
            }
        };
        if let Some(unused) = named
            .into_iter()
            .find(|key| !bound.used.contains(key.as_str()))
        {
            return Err(fail(format!("unknown material constant {unused}")));
        }
        Ok(Fragment {
            source: source(FRAGMENT, &expansion.combos, &bound.body, FRAGMENT_MAIN)?,
            vertex,
            reads: bound.reads,
            texcoord: bound.texcoord,
            inputs: bound.inputs,
            animated: bound.animated,
            constants: bound.constants,
            keys: bound.keys,
            unplaced,
            bindings,
            files,
        })
    }
}

/// Every use of an identifier, under another name.
/// Whether a uniform's annotation in `source` ties it to the material value `key`.
fn annotated(source: &str, key: &str) -> bool {
    let quoted = format!("\"{key}\"");
    source
        .lines()
        .any(|line| line.contains("\"material\"") && line.contains(&quoted))
}

fn rename(source: &str, from: &str, to: &str) -> Result<String, Error> {
    let mut out = String::with_capacity(source.len() + to.len() * 4);
    let mut at = 0;
    for token in parameters::tokens(source)?
        .iter()
        .filter(|token| token.text == from)
    {
        out.push_str(&source[at..token.start]);
        out.push_str(to);
        at = token.end;
    }
    out.push_str(&source[at..]);
    Ok(out)
}

/// Picks the source a stage's combos select, after making sure it doesn't redefine any of
/// the macros the host sets.
fn stage(
    output: &str,
    combos: &BTreeMap<String, i32>,
    controlled: &std::collections::HashSet<String>,
) -> Result<String, Error> {
    let mut blocked = false;
    for line in output.lines() {
        let visible = visible_line(line, &mut blocked);
        let Some(directive) = visible.trim().strip_prefix('#').map(str::trim_start) else {
            continue;
        };
        let (command, rest) = directive
            .split_once(char::is_whitespace)
            .unwrap_or((directive, ""));
        if matches!(command, "define" | "undef") {
            let name = rest
                .trim_start()
                .split(|character: char| character.is_whitespace() || character == '(')
                .next()
                .unwrap_or("");
            if combos.contains_key(name) || protected(name) || controlled.contains(name) {
                return Err(fail(format!(
                    "shader redefines host-controlled macro {name}"
                )));
            }
        }
    }
    conditional::select(output, combos)
}

/// A stage's whole source: the dialect, its combos, its body and the host's own main.
fn source(
    prelude: &str,
    combos: &BTreeMap<String, i32>,
    body: &str,
    main: &str,
) -> Result<String, Error> {
    let mut source = String::from(COMMON);
    source.push_str(prelude);
    for (name, value) in combos {
        source.push_str(&format!("#define {name} {value}\n"));
    }
    source.push_str(body);
    source.push_str(main);
    if source.len() > TOTAL_BYTES + 64 * 1024 {
        return Err(fail("prepared source exceeds its budget"));
    }
    Ok(source)
}

fn fail(message: impl std::fmt::Display) -> Error {
    Error::Format(format!("fragment shader: {message}"))
}

fn protected(name: &str) -> bool {
    name.starts_with("apolline_")
        || name.starts_with("g_")
        || name == "v_TexCoord"
        || matches!(
            name,
            "defined"
                | "main"
                | "uniform"
                | "varying"
                | "gl_FragColor"
                | "GLSL"
                | "HLSL"
                | "texSample2D"
                | "texSample2DLod"
                | "frac"
                | "saturate"
                | "atan2"
                | "ddx"
                | "ddy"
                | "CAST2"
                | "CAST3"
                | "CAST4"
                | "CAST3X3"
        )
}

fn identifier(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with("gl_")
        && !name.starts_with("apolline_")
        && name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic() || byte == b'_' || (index > 0 && byte.is_ascii_digit())
        })
}

fn integer(value: &Value) -> Result<i32, Error> {
    let value = crate::value::plain(value);
    value
        .as_i64()
        .and_then(|value| i32::try_from(value).ok())
        .or_else(|| value.as_bool().map(i32::from))
        .ok_or_else(|| fail("combo value must be a signed integer or boolean"))
}

fn path(parent: &str, name: &str) -> Result<String, Error> {
    if name.len() > 4096
        || name.is_empty()
        || name.contains(['\\', ':', '\0'])
        || Path::new(name).is_absolute()
    {
        return Err(fail("invalid shader resource path"));
    }
    let mut parts: Vec<_> = parent.split('/').collect();
    for part in Path::new(name).components() {
        match part {
            Component::Normal(part) => {
                parts.push(part.to_str().ok_or_else(|| fail("non-UTF-8 path"))?)
            }
            Component::CurDir => {}
            Component::ParentDir if parts.len() > 1 => {
                parts.pop();
            }
            _ => return Err(fail("include escapes the shaders directory")),
        }
    }
    Ok(parts.join("/"))
}

struct Expansion<F> {
    load: F,
    cache: HashMap<String, Option<Arc<str>>>,
    files: Vec<String>,
    stack: Vec<String>,
    inputs: usize,
    includes: usize,
    output: String,
    combos: BTreeMap<String, i32>,
}

impl<F: FnMut(&str) -> Result<Option<String>, Error>> Expansion<F> {
    fn read(&mut self, name: &str) -> Result<Option<Arc<str>>, Error> {
        if let Some(source) = self.cache.get(name) {
            return Ok(source.clone());
        }
        let source = (self.load)(name)?;
        if let Some(source) = &source {
            self.inputs += source.len();
            if source.len() > FILE_BYTES || self.inputs > TOTAL_BYTES || self.files.len() >= 64 {
                return Err(fail("source files exceed the preparation budget"));
            }
            self.files.push(name.to_owned());
        }
        let source = source.map(Arc::from);
        self.cache.insert(name.to_owned(), source.clone());
        Ok(source)
    }

    fn append(&mut self, text: &str) -> Result<(), Error> {
        if text.len() > TOTAL_BYTES.saturating_sub(self.output.len()) {
            return Err(fail("expanded source exceeds 1 MiB"));
        }
        self.output.push_str(text);
        Ok(())
    }

    fn expand(&mut self, name: &str) -> Result<(), Error> {
        if self.stack.len() >= 16 || self.stack.iter().any(|entry| entry == name) {
            return Err(fail(format!("include cycle or depth limit at {name}")));
        }
        let source = self
            .read(name)?
            .ok_or_else(|| fail(format!("missing {name}")))?;
        let id = self
            .files
            .iter()
            .position(|entry| entry == name)
            .unwrap_or(0)
            + 1;
        let hoisted = if self.stack.is_empty() {
            hoisted(&source)
        } else {
            Vec::new()
        };
        for &at in &hoisted {
            let line = source.lines().nth(at).unwrap_or_default();
            self.append(&format!("#line {} {id}\n{line}\n", at + 1))?;
        }
        self.append(&format!("#line 1 {id}\n"))?;
        self.stack.push(name.to_owned());
        let parent = name
            .rsplit_once('/')
            .map_or("shaders", |(parent, _)| parent);
        let mut blocked = false;
        for (line, original) in source.lines().enumerate() {
            if hoisted.contains(&line) {
                self.append("\n")?;
                continue;
            }
            if !blocked && let Some(annotation) = original.trim().strip_prefix("// [COMBO]") {
                let annotation: Value = serde_json::from_str(annotation)
                    .map_err(|error| fail(format!("{name}:{}: {error}", line + 1)))?;
                let combo = annotation
                    .get("combo")
                    .and_then(Value::as_str)
                    .filter(|name| identifier(name) && !protected(name))
                    .ok_or_else(|| fail("invalid combo name"))?;
                if self.combos.len() >= 64 {
                    return Err(fail("more than 64 combos"));
                }
                // An option list without a default starts at its first, 0.
                let value = annotation
                    .get("default")
                    .map(integer)
                    .transpose()?
                    .unwrap_or(0);
                if self.combos.insert(combo.to_owned(), value).is_some() {
                    return Err(fail(format!("duplicate combo {combo}")));
                }
            }
            let visible = visible_line(original, &mut blocked);
            let directive = visible.trim().strip_prefix('#').map(str::trim_start);
            if let Some(directive) = directive {
                let (command, rest) = directive
                    .split_once(char::is_whitespace)
                    .unwrap_or((directive, ""));
                if matches!(command, "version" | "extension" | "line") {
                    return Err(fail(format!(
                        "{name}:{}: #{command} is supplied by the host",
                        line + 1
                    )));
                }
                if command == "include" {
                    self.includes += 1;
                    if self.includes > 256 {
                        return Err(fail("more than 256 includes"));
                    }
                    let include = rest
                        .trim()
                        .strip_prefix('"')
                        .and_then(|rest| rest.strip_suffix('"'))
                        .ok_or_else(|| fail("include needs a quoted resource path"))?;
                    let local = path(parent, include)?;
                    let target = if self.read(&local)?.is_some() {
                        local
                    } else {
                        path("shaders", include)?
                    };
                    self.expand(&target)?;
                    self.append(&format!("#line {} {id}\n", line + 2))?;
                    continue;
                }
            }
            self.append(original)?;
            self.append("\n")?;
        }
        if blocked {
            return Err(fail(format!("{name}: unclosed block comment")));
        }
        self.stack.pop();
        Ok(())
    }
}

/// The lines of a shader's own uniforms after its first include, outside any conditional,
/// which go first: Wallpaper Engine's headers use the uniforms of the shaders that include
/// them, declared after the include.
fn hoisted(source: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut blocked = false;
    let mut depth = 0usize;
    let mut included = false;
    for (at, line) in source.lines().enumerate() {
        let visible = visible_line(line, &mut blocked);
        let visible = visible.trim();
        if let Some(directive) = visible.strip_prefix('#').map(str::trim_start) {
            let command = directive.split(char::is_whitespace).next().unwrap_or("");
            match command {
                "if" | "ifdef" | "ifndef" => depth += 1,
                "endif" => depth = depth.saturating_sub(1),
                "include" => included = true,
                _ => {}
            }
        } else if included
            && depth == 0
            && !blocked
            && visible.starts_with("uniform ")
            && visible.ends_with(';')
        {
            found.push(at);
        }
    }
    found
}

fn visible_line(line: &str, blocked: &mut bool) -> String {
    let mut output = String::new();
    let mut rest = line;
    while !rest.is_empty() {
        if *blocked {
            if let Some(end) = rest.find("*/") {
                rest = &rest[end + 2..];
                *blocked = false;
            } else {
                break;
            }
        } else if let Some(start) = rest.find("/*") {
            if let Some(comment) = rest.find("//").filter(|&comment| comment < start) {
                output.push_str(&rest[..comment]);
                break;
            }
            output.push_str(&rest[..start]);
            output.push(' ');
            rest = &rest[start + 2..];
            *blocked = true;
        } else {
            output.push_str(rest.split_once("//").map_or(rest, |(before, _)| before));
            break;
        }
    }
    output
}

const COMMON: &str = "\
#define GLSL 1
#define texSample2D texture
#define texSample2DLod textureLod
#define frac fract
#define saturate(x) clamp(x, 0.0, 1.0)
#define lerp mix
#define fmod(x, y) ((x) - (y) * trunc((x) / (y)))
#define mul(x, y) ((y) * (x))
#define atan2 atan
#define ddx dFdx
#define ddy dFdy
#define CAST2(x) vec2(x)
#define CAST3(x) vec3(x)
#define CAST4(x) vec4(x)
#define CAST3X3(x) mat3(x)
#define main apolline_effect_main
";

const FRAGMENT: &str = "\
#define varying in
#define gl_FragColor apolline_effect_color
out vec4 apolline_effect_color;
uniform vec4 apolline_layer_tint;
";

const FRAGMENT_MAIN: &str = "\n#undef main\nvoid main() { apolline_effect_main(); apolline_effect_color *= apolline_layer_tint; }\n";

/// The attributes are globals the host's main sets: the layer's corners in its own units,
/// centred, and their texture coordinates.
const VERTEX: &str = "\
#define varying out
uniform vec2 apolline_layer_size;
uniform vec2 apolline_uv_max;
vec3 a_Position;
vec2 a_TexCoord;
";

const VERTEX_MAIN: &str = "
#undef main
void main() {
    vec2 apolline_corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    a_Position = vec3((apolline_corner - 0.5) * apolline_layer_size, 0.0);
    a_TexCoord = vec2(apolline_corner.x, 1.0 - apolline_corner.y) * apolline_uv_max;
    apolline_effect_main();
}
";
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn prepare(source: &str, combos: Value, constants: Value) -> Result<Fragment, Error> {
        Fragment::prepare(
            "demo.frag",
            combos.as_object().unwrap(),
            constants.as_object().unwrap(),
            |path| Ok((path == "shaders/demo.frag").then(|| source.to_owned())),
        )
    }

    #[test]
    fn helpers_may_hand_back_values_through_out_parameters_but_shaders_get_one_output() {
        let helper = "void split(vec4 c, out vec3 rgb, out float a) { rgb = c.rgb; a = c.a; }\nvoid main() { vec3 rgb; float a; split(vec4(1.0), rgb, a); gl_FragColor = vec4(lerp(rgb, vec3(fmod(-3.0, 2.0)), 0.5), a); }";
        assert!(prepare(helper, json!({}), json!({})).is_ok());
        let second = "out vec4 extra;\nvoid main() { gl_FragColor = vec4(1.0); }";
        assert!(prepare(second, json!({}), json!({})).is_err());
    }

    #[test]
    fn every_sampler_and_resolution_slot_read_must_be_bound() {
        let fragment = prepare("uniform sampler2D g_Texture0;\nuniform sampler2D g_Texture1;\nuniform vec4 g_Texture7Resolution;\nvoid main() { gl_FragColor = texture2D(g_Texture1, vec2(0.0)) + g_Texture7Resolution; }", json!({}), json!({})).unwrap();
        assert!(fragment.validate_inputs(1).is_err());
        assert!(fragment.validate_inputs(127).is_err());
        assert!(fragment.validate_inputs(255).is_ok());
        assert!(fragment.validate_inputs(0).is_err());
        // Slots only declared, as stock shaders often leave them, need nothing bound.
        let declared = prepare("uniform sampler2D g_Texture0;\nuniform sampler2D g_Texture1;\nuniform vec4 g_Texture7Resolution;\nvoid main() { gl_FragColor = vec4(1.0); }", json!({}), json!({})).unwrap();
        assert!(declared.validate_inputs(1).is_ok());
    }

    #[test]
    fn includes_combos_and_numeric_constants_form_a_complete_static_shader() {
        let source = "// [COMBO] {\"combo\":\"GRAY\",\"default\":0}\n#include \"tone.h\"\nvarying vec2 v_TexCoord;\nuniform sampler2D g_Texture0;\nuniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord) * u_Amount; }";
        let fragment = Fragment::prepare(
            "demo.frag",
            json!({"GRAY":1}).as_object().unwrap(),
            json!({"amount":0.5}).as_object().unwrap(),
            |path| {
                Ok(match path {
                    "shaders/demo.frag" => Some(source.into()),
                    "shaders/tone.h" => {
                        Some("float half_value(float value) { return value * 0.5; }".into())
                    }
                    _ => None,
                })
            },
        )
        .unwrap();
        assert!(fragment.source.contains("#define GRAY 1"));
        assert!(fragment.source.contains("uniform float u_Amount;"));
        assert_eq!(
            fragment.constants,
            [(
                "u_Amount".to_owned(),
                Constant::Float {
                    bits: [0.5f32.to_bits(), 0, 0, 0],
                    count: 1
                }
            )]
        );
        assert!(fragment.source.contains("float half_value"));
        // The shader's own uniforms come before the header, which may read them.
        let at = |text: &str| fragment.source.find(text).unwrap();
        assert!(at("uniform float u_Amount;") < at("float half_value"));
        assert!(!fragment.source.contains("#include"));
        assert_eq!(fragment.texcoord, 2);
        assert_eq!(fragment.files, ["shaders/demo.frag", "shaders/tone.h"]);
    }

    #[test]
    fn include_cycles_missing_sources_and_escape_attempts_are_errors() {
        for source in [
            "#include \"demo.frag\"",
            "#include \"missing.h\"",
            "#include \"../outside.h\"",
        ] {
            assert!(prepare(source, json!({}), json!({})).is_err());
        }
        assert!(prepare("#version 330", json!({}), json!({})).is_err());
        assert!(prepare("#define main replacement", json!({}), json!({})).is_err());
        assert!(prepare("/* unclosed", json!({}), json!({})).is_err());
    }

    #[test]
    fn comments_do_not_trigger_includes_or_uniforms() {
        let fragment = prepare("/*\n#include \"missing.h\"\nuniform float g_Time;\n*/\n// uniform sampler2D g_Texture1;\nvoid main() { gl_FragColor = vec4(1.0); }", json!({}), json!({})).unwrap();
        assert_eq!(fragment.files.len(), 1);
    }

    #[test]
    fn unsupported_dynamic_inputs_and_ignored_parameters_are_refused() {
        for declaration in [
            "uniform vec4 g_LightsColor;",
            "uniform sampler2D g_Texture8;",
            "uniform sampler2D g_Texture0; // {\"combo\":\"MASK\"}",
            "varying vec2 customUV;",
            "uniform float values[4];",
        ] {
            assert!(prepare(declaration, json!({}), json!({})).is_err());
        }
        // A combo the shader doesn't declare is still defined, as the engine defines it.
        let undeclared = prepare("void main() {}", json!({"UNKNOWN":1}), json!({})).unwrap();
        assert!(undeclared.source.contains("#define UNKNOWN 1"));
        assert!(prepare("void main() {}", json!({}), json!({"ignored":1})).is_ok());
        assert!(prepare("uniform float amount;", json!({}), json!({"amount":"nan"})).is_err());
        assert!(prepare("uniform vec3 color;", json!({}), json!({"color":[1,2]})).is_err());
    }

    #[test]
    fn only_selected_declarations_contribute_uniforms_and_texture_slots() {
        let source = "// [COMBO] {\"combo\":\"LIVE\",\"default\":0}\n#if LIVE\nuniform vec4 g_LightsColor;\n#else\nvarying vec4 v_TexCoord;\nuniform sampler2D g_Texture0;\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord.xy); }\n#endif\n#if 0\nuniform sampler2D g_Texture7;\ninvalid syntax $ ! ;\n#endif";
        let selected = prepare(source, json!({}), json!({})).unwrap();
        assert_eq!(selected.texcoord, 4);
        assert!(selected.validate_inputs(1).is_ok());
        assert!(!selected.source.contains("g_LightsColor"));
        assert!(!selected.source.contains("g_Texture7"));
        assert!(prepare(source, json!({"LIVE":1}), json!({})).is_err());
        // A value nothing declares is let be.
        assert!(prepare(source, json!({}), json!({"amount":0.5})).is_ok());
    }

    #[test]
    fn engine_uniforms_and_compatibility_macros_cannot_be_aliased() {
        for name in [
            "g_Texture0",
            "g_Texture0Resolution",
            "v_TexCoord",
            "atan2",
            "defined",
        ] {
            assert!(
                prepare(
                    &format!("#define {name} other\nvoid main() {{}}"),
                    json!({}),
                    json!({})
                )
                .is_err()
            );
            assert!(
                prepare(
                    &format!("// [COMBO] {{\"combo\":\"{name}\",\"default\":1}}\nvoid main() {{}}"),
                    json!({}),
                    json!({})
                )
                .is_err()
            );
        }
        assert!(prepare("#line 1 99\nvoid main() {}", json!({}), json!({})).is_err());
    }

    #[test]
    fn framebuffer_constants_are_supported_with_exact_types() {
        let supported = "uniform vec3 g_Screen;\nuniform vec2 g_TexelSize;\nuniform vec2 g_TexelSizeHalf;\nvoid main() { gl_FragColor = vec4(g_TexelSize, g_TexelSizeHalf.x, g_Screen.z); }";
        assert!(prepare(supported, json!({}), json!({})).is_ok());
        for declaration in [
            "uniform vec2 g_Screen;",
            "uniform float g_TexelSize;",
            "uniform vec3 g_TexelSizeHalf;",
        ] {
            assert!(prepare(declaration, json!({}), json!({})).is_err());
        }
        // Textures are never shrunk here, so the reduction is none.
        let reduced = prepare("uniform float g_TextureReductionScale;\nvoid main() { gl_FragColor = vec4(g_TextureReductionScale); }", json!({}), json!({})).unwrap();
        assert!(
            reduced
                .source
                .contains("const float g_TextureReductionScale = 1.0;")
        );
    }

    #[test]
    fn metadata_defaults_are_not_changed_by_material_values_from_other_preparations() {
        let source = "uniform vec3 u_Color; // {\"material\":\"color\",\"default\":\"1 0 0\"}\nvoid main() { gl_FragColor = vec4(u_Color, 1.0); }";
        let default = prepare(source, json!({}), json!({})).unwrap();
        let edited = prepare(source, json!({}), json!({"color":"0 1 0"})).unwrap();
        assert_eq!(default.source, edited.source);
        assert_ne!(default.constants, edited.constants);
        assert_eq!(prepare(source, json!({}), json!({})).unwrap(), default);
    }

    fn staged(
        vertex: &str,
        fragment: &str,
        combos: Value,
        constants: Value,
    ) -> Result<Fragment, Error> {
        let load = |path: &str| {
            Ok(match path {
                "shaders/demo.vert" => Some(vertex.to_owned()),
                "shaders/demo.frag" => Some(fragment.to_owned()),
                _ => None,
            })
        };
        let vertex = Template::load("demo.vert", load)?;
        Template::load("demo.frag", load)?.prepare(
            combos.as_object().unwrap(),
            constants.as_object().unwrap(),
            Some(1),
            Some(vertex),
        )
    }

    #[test]
    fn a_vertex_stage_shares_combos_values_and_varyings_with_its_fragment() {
        let vertex = "// [COMBO] {\"combo\":\"SCROLL\",\"default\":0}\nattribute vec3 a_Position;\nattribute vec2 a_TexCoord;\nuniform mat4 g_ModelViewProjectionMatrix;\nuniform float g_Time;\nuniform float u_Speed; // {\"material\":\"speed\",\"default\":1}\nvarying vec4 v_TexCoord;\nvarying float v_Fade;\nvoid main() {\n gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix);\n#if SCROLL\n v_TexCoord = vec4(a_TexCoord + g_Time * u_Speed, a_TexCoord);\n#else\n v_TexCoord = vec4(a_TexCoord, a_TexCoord);\n#endif\n v_Fade = 1.0;\n}";
        let fragment = "uniform sampler2D g_Texture0;\nvarying vec4 v_TexCoord;\nvarying float v_Fade;\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord.xy) * v_Fade; }";
        let prepared = staged(vertex, fragment, json!({"SCROLL": 1}), json!({"speed": 2})).unwrap();
        let source = prepared.vertex.as_deref().unwrap();
        assert!(source.contains("#define SCROLL 1"));
        assert!(!source.contains("attribute"));
        assert!(prepared.animated);
        assert_eq!(prepared.constants[0].0, "u_Speed");

        for (vertex, fragment, why) in [
            ("void main() {}", fragment, "isn't written"),
            (
                "varying vec2 v_TexCoord;\nvarying float v_Fade;\nvoid main() {}",
                fragment,
                "isn't written",
            ),
            (
                "uniform mat4 g_EffectTextureProjectionMatrix;\nvoid main() {}",
                "void main() {}",
                "unsupported uniform",
            ),
            (
                "uniform sampler2D g_Texture0;\nvoid main() {}",
                "void main() {}",
                "sample textures",
            ),
            (
                "attribute vec4 a_Color;\nvoid main() {}",
                "void main() {}",
                "attribute",
            ),
            (
                "void main() { gl_FragColor = vec4(1.0); }",
                "void main() {}",
                "reserved",
            ),
            (
                "void main() {}",
                "void main() { gl_Position = vec4(1.0); }",
                "reserved",
            ),
        ] {
            let error = staged(vertex, fragment, json!({}), json!({}))
                .unwrap_err()
                .to_string();
            assert!(error.contains(why), "{why}: {error}");
        }
        // A value no uniform names is let be; one a declaration names but nothing read isn't.
        assert!(
            staged(
                "void main() {}",
                "void main() {}",
                json!({}),
                json!({"speed": 2})
            )
            .is_ok()
        );
        let unread = staged(
            "void main() {}",
            "// uniform float g_Speed; // {\"material\":\"speed\",\"default\":1}\nvoid main() {}",
            json!({}),
            json!({"speed": 2}),
        );
        assert!(
            unread
                .unwrap_err()
                .to_string()
                .contains("unknown material constant")
        );
    }

    #[test]
    fn stages_keep_their_own_values_and_unread_varyings_are_let_be() {
        let vertex = "uniform float g_Speed; // {\"material\":\"speed\",\"default\":1}\nvarying vec2 v_Other;\nvoid main() { v_Other = vec2(g_Speed); }";
        let fragment = "uniform float g_Speed; // {\"material\":\"speeduv\",\"default\":5}\nvarying vec2 v_Other;\nvarying vec2 v_Scroll;\nvoid main() { gl_FragColor = vec4(v_Other, g_Speed, 1.0); }";
        let prepared = staged(vertex, fragment, json!({}), json!({})).unwrap();
        let names: Vec<&str> = prepared
            .constants
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["g_Speed", "apolline_vertex_g_Speed"]);
        assert!(
            prepared
                .vertex
                .unwrap()
                .contains("vec2(apolline_vertex_g_Speed)")
        );
        let reading = fragment.replace("v_Other, g_Speed", "v_Scroll, g_Speed");
        assert!(staged(vertex, &reading, json!({}), json!({})).is_err());
    }

    #[test]
    fn stock_values_may_carry_the_engine_prefix_when_tied_to_the_material() {
        let source = "uniform float g_Speed; // {\"material\":\"speed\",\"default\":2}\nvoid main() { gl_FragColor = vec4(g_Speed); }";
        let fragment = prepare(source, json!({}), json!({"speed": 3})).unwrap();
        assert_eq!(fragment.constants[0].0, "g_Speed");
        assert_eq!(fragment.constants[0].1.floats()[0], 3.0);
        assert!(
            prepare(
                "uniform float g_Speed;\nvoid main() {}",
                json!({}),
                json!({})
            )
            .is_err()
        );
    }

    #[test]
    fn engine_inputs_are_named_in_what_a_shader_reads() {
        let reads =
            |source: &str| prepare(source, json!({}), json!({})).map(|fragment| fragment.reads);
        let all = reads("uniform vec2 g_PointerPosition;\nuniform float g_Daytime;\nuniform float g_AudioSpectrum16Left[16];\nuniform float g_AudioSpectrum64Right[ 64 ];\nvoid main() { gl_FragColor = vec4(g_PointerPosition, g_Daytime, g_AudioSpectrum16Left[0] + g_AudioSpectrum64Right[63]); }").unwrap();
        assert!(all.pointer && all.daytime && all.audio && !all.parallax);
        assert_eq!(reads("void main() {}").unwrap(), Reads::default());
        for refused in [
            "uniform float g_AudioSpectrum16Left[32];",
            "uniform float g_AudioSpectrum12Left[12];",
            "uniform float u_Values[4];",
            "uniform vec2 g_Daytime;",
            "uniform float g_AudioSpectrum16Left[16];\nuniform float g_AudioSpectrum16Left[16];",
        ] {
            assert!(
                reads(&format!("{refused}\nvoid main() {{}}")).is_err(),
                "{refused}"
            );
        }
    }

    #[test]
    fn repeated_includes_cannot_expand_without_a_bound() {
        let source = "#include \"large.h\"\n".repeat(80);
        let result = Fragment::prepare("demo.frag", &Map::new(), &Map::new(), |path| {
            Ok(match path {
                "shaders/demo.frag" => Some(source.clone()),
                "shaders/large.h" => Some(" ".repeat(20000)),
                _ => None,
            })
        });
        assert!(result.err().unwrap().to_string().contains("1 MiB"));
        let oversized = " ".repeat(FILE_BYTES + 1);
        assert!(prepare(&oversized, json!({}), json!({})).is_err());
    }
}
