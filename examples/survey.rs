//! What real Wallpaper Engine projects are made of, without what's in them:
//! `cargo run --release --example survey -- <folder of projects> [assets folder] > survey.md`.
//!
//! Run it over a Workshop folder (`steamapps/workshop/content/431960`) and Wallpaper Engine's
//! `assets`. It reports the shapes of the files: which keys appear where and how often, which
//! effects, shaders, particle components and blend modes are used, texture header numbers,
//! why scenes aren't drawn here, and what to support next. It never reports shader text,
//! titles, file contents or pixels, so the report can be shared.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use apolline_scene::Project;
use apolline_scene::pkg::Package;
use serde_json::Value;

/// Key paths seen, by file kind, with the value types each had and how often.
type Shapes = BTreeMap<String, BTreeMap<String, usize>>;

#[derive(Default)]
struct Survey {
    projects: usize,
    kinds: BTreeMap<String, usize>,
    scenes: usize,
    prepared: usize,
    /// Why each scene that didn't prepare didn't, as reason codes.
    blocked: Vec<BTreeSet<String>>,
    errors: BTreeMap<String, usize>,
    shapes: BTreeMap<&'static str, Shapes>,
    /// Public vocabulary: effect folders, shader names, particle components, blend modes.
    words: BTreeMap<&'static str, BTreeMap<String, usize>>,
    textures: BTreeMap<String, usize>,
    models: BTreeMap<String, usize>,
    /// Why layers are left out, in layers and in scenes.
    layers: BTreeMap<String, (usize, usize)>,
    /// Values of plain settings: booleans, numbers and a few named choices.
    values: BTreeMap<String, BTreeMap<String, usize>>,
}

/// String settings whose values are a fixed choice, not content.
const CHOICES: &[&str] = &[
    "alignment",
    "depthtest",
    "horizontalalign",
    "playbackmode",
    "shape",
    "verticalalign",
];

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(root) = args.next().map(PathBuf::from) else {
        eprintln!("usage: survey <folder of projects> [assets folder]");
        std::process::exit(2);
    };
    let assets = args.next().map(PathBuf::from);
    if let Some(assets) = assets.as_ref().filter(|assets| !assets.is_dir()) {
        eprintln!("survey: {} isn't a folder", assets.display());
        std::process::exit(1);
    }
    let mut survey = Survey::default();
    let mut folders: Vec<PathBuf> = match std::fs::read_dir(&root) {
        Ok(entries) => entries.flatten().map(|entry| entry.path()).collect(),
        Err(error) => {
            eprintln!("survey: can't read {}: {error}", root.display());
            std::process::exit(1);
        }
    };
    folders.sort();
    for folder in folders
        .iter()
        .filter(|folder| folder.join("project.json").is_file())
    {
        survey.project(folder, assets.as_deref());
    }
    print!("{}", survey.report());
}

impl Survey {
    fn project(&mut self, folder: &Path, assets: Option<&Path>) {
        self.projects += 1;
        let project: Value = std::fs::read_to_string(folder.join("project.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        let kind = project
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_lowercase();
        *self.kinds.entry(kind.clone()).or_default() += 1;
        shape(&project, "", self.shapes.entry("project.json").or_default());
        if kind != "scene" {
            return;
        }
        self.scenes += 1;
        let opened = match Project::open(folder, assets) {
            Ok(opened) => opened,
            Err(err) => {
                *self.errors.entry(class(&err.to_string())).or_default() += 1;
                return;
            }
        };
        let scene_file = project
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or("scene.json");
        let scene_json = json_of(&opened, scene_file);
        if let Some(scene) = &scene_json {
            shape(scene, "", self.shapes.entry("scene.json").or_default());
            self.objects(&opened, scene);
        }
        self.files(folder);
        if let Some(scene) = &scene_json {
            self.settings(scene);
        }
        let mut reasons = BTreeSet::new();
        if let Ok(scene) = opened.scene() {
            for missing in scene.missing() {
                reasons.insert(match &missing {
                    apolline_scene::Missing::UnknownKey(path)
                    | apolline_scene::Missing::Animated(path)
                    | apolline_scene::Missing::Scripted(path) => {
                        format!("{}:{}", missing.code(), unindexed(path))
                    }
                    apolline_scene::Missing::Setting(what) => format!("setting:{what}"),
                    other => other.code().to_owned(),
                });
            }
            // Every layer, whatever else the scene needs, so what layers lack shows too.
            let (_, skipped) = opened.layers(&scene);
            let mut seen = BTreeSet::new();
            for reason in skipped.iter().filter(|reason| !reason.ends_with("omitted")) {
                let reason = format!("layer:{}", layer_class(reason));
                let counts = self.layers.entry(reason.clone()).or_default();
                counts.0 += 1;
                if seen.insert(reason.clone()) {
                    counts.1 += 1;
                }
                reasons.insert(reason);
            }
            // Every script, whatever else the scene needs, so what scripts lack shows too.
            if let Err(error) = opened.stage(&scene) {
                for failure in error.to_string().split("; ") {
                    let reason = format!("script:{}", script_class(failure));
                    let counts = self.layers.entry(reason.clone()).or_default();
                    counts.0 += 1;
                    if seen.insert(reason.clone()) {
                        counts.1 += 1;
                    }
                    reasons.insert(reason);
                }
            }
        }
        // As apolline-wall prepares them: it moves the pointer.
        match opened.prepare_pointed() {
            Ok(_) => self.prepared += 1,
            Err(err) => {
                let class = class(&err.to_string());
                *self.errors.entry(class.clone()).or_default() += 1;
                if !class.starts_with("needs") || reasons.is_empty() {
                    reasons.insert(format!("prepare:{class}"));
                }
                self.blocked.push(reasons);
            }
        }
    }

    /// Plain values of the scene's general settings and its objects' own keys.
    fn settings(&mut self, scene: &Value) {
        let general = scene.get("general").and_then(Value::as_object);
        for (key, value) in general.into_iter().flatten() {
            self.value(&format!("general.{key}"), key, value);
        }
        let objects = scene.get("objects").and_then(Value::as_array);
        for object in objects.into_iter().flatten().filter_map(Value::as_object) {
            for (key, value) in object {
                if !matches!(key.as_str(), "id" | "parent") {
                    self.value(&format!("objects[].{key}"), key, value);
                }
            }
        }
    }

    fn value(&mut self, path: &str, key: &str, value: &Value) {
        let value = match value {
            Value::Object(fields) if fields.keys().all(|key| key == "user" || key == "value") => {
                fields.get("value").unwrap_or(&Value::Null)
            }
            other => other,
        };
        let shown = match value {
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => number
                .as_f64()
                .map(|number| format!("{}", (number * 1000.0).round() / 1000.0))
                .unwrap_or_default(),
            Value::String(text) if CHOICES.contains(&key) && text.len() <= 32 => vocabulary(text),
            _ => return,
        };
        *self
            .values
            .entry(path.to_owned())
            .or_default()
            .entry(shown)
            .or_default() += 1;
    }

    /// Effects, materials, shaders, models and particle systems the scene's objects use.
    fn objects(&mut self, project: &Project, scene: &Value) {
        let objects = scene.get("objects").and_then(Value::as_array);
        for object in objects.into_iter().flatten() {
            if let Some(model) = object.get("image").and_then(Value::as_str)
                && let Some(model) = json_of(project, model)
            {
                shape(&model, "", self.shapes.entry("model").or_default());
                if let Some(material) = model.get("material").and_then(Value::as_str) {
                    self.material(project, material);
                }
                if model.get("puppet").is_some() {
                    self.word("model feature", "puppet");
                }
            }
            for effect in object
                .get("effects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                shape(
                    effect,
                    "",
                    self.shapes.entry("effect instance").or_default(),
                );
                let Some(file) = effect.get("file").and_then(Value::as_str) else {
                    continue;
                };
                self.word("effect", &effect_name(file));
                if let Some(definition) = json_of(project, file) {
                    shape(
                        &definition,
                        "",
                        self.shapes.entry("effect.json").or_default(),
                    );
                    for pass in definition
                        .get("passes")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(material) = pass.get("material").and_then(Value::as_str) {
                            self.material(project, material);
                        }
                    }
                }
            }
            if let Some(file) = object.get("particle").and_then(Value::as_str)
                && let Some(system) = json_of(project, file)
            {
                shape(
                    &system,
                    "",
                    self.shapes.entry("particle system").or_default(),
                );
                for part in ["emitter", "initializer", "operator", "renderer"] {
                    for component in system
                        .get(part)
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(name) = component.get("name").and_then(Value::as_str) {
                            self.word(
                                "particle component",
                                &format!("{part}: {}", vocabulary(name)),
                            );
                        }
                    }
                }
                if let Some(material) = system.get("material").and_then(Value::as_str) {
                    self.material(project, material);
                }
            }
        }
    }

    fn material(&mut self, project: &Project, name: &str) {
        let Some(material) = json_of(project, name) else {
            return;
        };
        shape(&material, "", self.shapes.entry("material").or_default());
        for pass in material
            .get("passes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(shader) = pass.get("shader").and_then(Value::as_str) {
                self.word("shader", &vocabulary(shader));
            }
            if let Some(blending) = pass.get("blending").and_then(Value::as_str) {
                self.word("blending", &vocabulary(blending));
            }
        }
    }

    /// Texture headers and model magic in the project's own files.
    fn files(&mut self, folder: &Path) {
        let mut names: Vec<(String, Vec<u8>)> = Vec::new();
        if let Ok(package) = Package::open(&folder.join("scene.pkg")) {
            for name in package.names() {
                if (name.ends_with(".tex") || name.ends_with(".mdl"))
                    && let Some(bytes) = package.get(name)
                {
                    names.push((name.to_owned(), bytes[..bytes.len().min(4096)].to_vec()));
                }
            }
        }
        walk(folder, 0, &mut names);
        for (name, head) in names {
            if name.ends_with(".tex") {
                *self.textures.entry(texture_header(&head)).or_default() += 1;
            } else {
                let magic: String = head
                    .iter()
                    .take(16)
                    .take_while(|byte| byte.is_ascii_graphic())
                    .map(|&byte| char::from(byte))
                    .collect();
                *self.models.entry(magic).or_default() += 1;
            }
        }
    }

    fn word(&mut self, kind: &'static str, word: &str) {
        *self
            .words
            .entry(kind)
            .or_default()
            .entry(word.to_owned())
            .or_default() += 1;
    }

    fn report(&self) -> String {
        let mut out = String::from("# apolline-scene survey\n\n");
        out += &format!(
            "{} projects; {} scenes, {} prepare natively.\n\n",
            self.projects, self.scenes, self.prepared
        );
        out += "## Project types\n\n";
        out += &table(&self.kinds);
        out += "## What to support next\n\n";
        out += "A scene draws only when all of it does. Each line is a reason, the scenes it alone \
                blocks, and the scenes it is one of the reasons for.\n\n";
        let mut alone: BTreeMap<String, usize> = BTreeMap::new();
        let mut any: BTreeMap<String, usize> = BTreeMap::new();
        for reasons in &self.blocked {
            for reason in reasons {
                *any.entry(reason.clone()).or_default() += 1;
            }
            if reasons.len() == 1
                && let Some(reason) = reasons.iter().next()
            {
                *alone.entry(reason.clone()).or_default() += 1;
            }
        }
        let mut ranked: Vec<(&String, &usize)> = any.iter().collect();
        ranked.sort_by(|a, b| {
            let key = |(reason, count): &(&String, &usize)| {
                (alone.get(*reason).copied().unwrap_or(0), **count)
            };
            key(b).cmp(&key(a))
        });
        out += "| reason | alone | among |\n|---|---|---|\n";
        for (reason, count) in ranked.into_iter().take(60) {
            out += &format!(
                "| {reason} | {} | {count} |\n",
                alone.get(reason).copied().unwrap_or(0)
            );
        }
        out += "\n## Scenes a few steps away\n\nScenes that only up to three reasons keep from \
                drawing, by what those reasons are.\n\n";
        let mut near: BTreeMap<String, usize> = BTreeMap::new();
        for reasons in self.blocked.iter().filter(|reasons| reasons.len() <= 3) {
            let set: Vec<&str> = reasons.iter().map(String::as_str).collect();
            *near.entry(set.join(" + ")).or_default() += 1;
        }
        let mut near: Vec<(String, usize)> = near.into_iter().collect();
        near.sort_by(|a, b| {
            let steps = |set: &str| set.matches(" + ").count();
            steps(&a.0).cmp(&steps(&b.0)).then(b.1.cmp(&a.1))
        });
        out += "| reasons | scenes |\n|---|---|\n";
        for (set, count) in near {
            out += &format!("| {set} | {count} |\n");
        }
        out += "\n## Layers left out\n\nWhy single layers can't be drawn, in layers and in scenes.\n\n";
        out += "| reason | layers | scenes |\n|---|---|---|\n";
        let mut layers: Vec<(&String, &(usize, usize))> = self.layers.iter().collect();
        layers.sort_by(|a, b| (b.1).1.cmp(&(a.1).1).then((b.1).0.cmp(&(a.1).0)));
        for (reason, (count, scenes)) in layers.into_iter().take(80) {
            out += &format!("| {reason} | {count} | {scenes} |\n");
        }
        out += "\n## Setting values\n\nThe most common values of plain settings.\n\n| setting | values |\n|---|---|\n";
        for (path, values) in &self.values {
            let mut values: Vec<(&String, &usize)> = values.iter().collect();
            values.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let shown: Vec<String> = values
                .iter()
                .take(8)
                .map(|(value, count)| format!("{value} ×{count}"))
                .collect();
            out += &format!("| `{path}` | {} |\n", shown.join(", "));
        }
        out += "\n## Errors by class\n\n";
        out += &table(&self.errors);
        for (kind, words) in &self.words {
            out += &format!("## {kind}\n\n");
            out += &table(words);
        }
        out += "## Texture headers\n\n";
        out += &table(&self.textures);
        out += "## Model magic\n\n";
        out += &table(&self.models);
        for (file, shapes) in &self.shapes {
            out += &format!("## Shapes: {file}\n\n| key | types |\n|---|---|\n");
            for (path, types) in shapes {
                let types: Vec<String> = types
                    .iter()
                    .map(|(kind, count)| format!("{kind} ×{count}"))
                    .collect();
                out += &format!("| `{path}` | {} |\n", types.join(", "));
            }
            out += "\n";
        }
        out
    }
}

fn json_of(project: &Project, name: &str) -> Option<Value> {
    serde_json::from_slice(&project.file(name)?).ok()
}

/// Records each key path in a value with its type, arrays folded to `[]`.
fn shape(value: &Value, path: &str, shapes: &mut Shapes) {
    let kind = match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    if !path.is_empty() {
        *shapes
            .entry(path.to_owned())
            .or_default()
            .entry(kind.to_owned())
            .or_default() += 1;
    }
    if shapes.len() > 20_000 || path.len() > 256 {
        return;
    }
    match value {
        Value::Object(fields) => {
            for (key, value) in fields {
                let key = if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    key.as_str()
                } else {
                    "<key>"
                };
                shape(value, &format!("{path}.{key}"), shapes);
            }
        }
        Value::Array(items) => {
            for item in items {
                shape(item, &format!("{path}[]"), shapes);
            }
        }
        _ => {}
    }
}

/// `effects/waterripple/effect.json` → `waterripple`.
fn effect_name(file: &str) -> String {
    match file
        .strip_prefix("effects/")
        .and_then(|rest| rest.split('/').next())
    {
        Some(name) => vocabulary(name),
        None => "<custom>".into(),
    }
}

/// Names Wallpaper Engine's own vocabulary is made of; anything else is folded away.
fn vocabulary(word: &str) -> String {
    if word.len() <= 64
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.'))
    {
        word.to_owned()
    } else {
        "<other>".into()
    }
}

fn unindexed(path: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for c in path.chars() {
        match c {
            '[' => {
                skipping = true;
                out.push_str("[]");
            }
            ']' => skipping = false,
            _ if skipping => {}
            _ => out.push(c),
        }
    }
    out
}

/// A left-out layer's reason without the layer's name, which comes first.
fn layer_class(message: &str) -> String {
    let reason = message
        .split_once(": ")
        .map_or(message, |(_, reason)| reason);
    let words: Vec<String> = reason
        .split_whitespace()
        .filter(|word| !word.contains('/') && !word.contains('.'))
        .map(|word| word.chars().filter(char::is_ascii_alphanumeric).collect())
        .filter(|word: &String| !word.is_empty())
        .take(9)
        .collect();
    let joined: String = words.join("_").chars().take(64).collect();
    vocabulary(&joined)
}

/// A failed script as what it asked for that isn't here, or what went wrong, without the
/// layer's name.
fn script_class(failure: &str) -> String {
    if let Some((_, asked)) = failure.split_once("(it asked for ") {
        let members: Vec<&str> = asked.trim_end_matches(')').split(", ").take(3).collect();
        return vocabulary(&members.join("+").chars().take(64).collect::<String>());
    }
    let reason = failure.rsplit("': ").next().unwrap_or(failure);
    let reason = reason.split_once(": ").map_or(reason, |(_, rest)| rest);
    let words: Vec<String> = reason
        .split_whitespace()
        .map(|word| word.chars().filter(char::is_ascii_alphanumeric).collect())
        .filter(|word: &String| !word.is_empty())
        .take(6)
        .collect();
    vocabulary(&words.join("_").chars().take(64).collect::<String>())
}

/// An error's first words, without paths or names. Layer errors come as `name: reason; …`,
/// so only the first reason after its last colon is kept.
fn class(message: &str) -> String {
    let first = message.split("; ").next().unwrap_or(message);
    let reason = first.rsplit(": ").next().unwrap_or(first);
    let words: Vec<String> = reason
        .split_whitespace()
        .filter(|word| !word.contains('/') && !word.contains('.'))
        .map(|word| word.chars().filter(char::is_ascii_alphanumeric).collect())
        .filter(|word: &String| !word.is_empty())
        .take(6)
        .collect();
    vocabulary(&words.join("_"))
}

/// Container tags, format and flags of a `.tex`, and whether an animation table follows.
fn texture_header(head: &[u8]) -> String {
    let tag = |at: usize| {
        head.get(at..at + 8)
            .filter(|tag| tag.iter().all(u8::is_ascii_alphanumeric))
            .map(|tag| String::from_utf8_lossy(tag).into_owned())
    };
    let number = |at: usize| {
        head.get(at..at + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let (Some(version), Some(info)) = (tag(0), tag(9)) else {
        return "unreadable".into();
    };
    let format = number(18).unwrap_or(u32::MAX);
    let flags = number(22).unwrap_or(u32::MAX);
    let container = tag(46).unwrap_or_else(|| "?".into());
    let frames = head
        .windows(4)
        .position(|window| window == b"TEXS")
        .and_then(tag)
        .unwrap_or_else(|| "-".into());
    format!("{version} {info} {container} format {format} flags {flags:#x} frames {frames}")
}

/// Counts as a table, most common first.
fn table(counts: &BTreeMap<String, usize>) -> String {
    let mut rows: Vec<(&String, &usize)> = counts.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let mut out = String::from("| | count |\n|---|---|\n");
    for (name, count) in rows {
        out += &format!("| {name} | {count} |\n");
    }
    out + "\n"
}

/// `.tex` and `.mdl` files under a folder project, first 4 KiB of each.
fn walk(folder: &Path, depth: usize, found: &mut Vec<(String, Vec<u8>)>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() && depth < 6 {
            walk(&path, depth + 1, found);
        } else if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("tex" | "mdl")
        ) && found.len() < 10_000
        {
            let mut head = Vec::new();
            if let Ok(file) = std::fs::File::open(&path) {
                use std::io::Read;
                let _ = file.take(4096).read_to_end(&mut head);
            }
            found.push((path.to_string_lossy().into_owned(), head));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_but_vocabulary_gets_out() {
        assert_eq!(
            unindexed("objects[12].effects[3].file"),
            "objects[].effects[].file"
        );
        assert_eq!(
            effect_name("effects/waterripple/effect.json"),
            "waterripple"
        );
        assert_eq!(effect_name("my effect.json"), "<custom>");
        assert_eq!(vocabulary("Лето у моря"), "<other>");
        assert_eq!(class("can't read /home/me/x.tex"), "cant_read");
        assert_eq!(
            class("My Secret Layer: effect x: unsupported field fbos; b: c"),
            "unsupported_field_fbos"
        );
        assert_eq!(texture_header(b"junk"), "unreadable");
        let mut shapes = Shapes::new();
        shape(
            &serde_json::json!({"title": "secret", "my key!": 1, "list": [{"a": true}]}),
            "",
            &mut shapes,
        );
        let keys: Vec<&str> = shapes.keys().map(String::as_str).collect();
        assert_eq!(keys, [".<key>", ".list", ".list[]", ".list[].a", ".title"]);
    }
}
