//! Fonts that aren't the project's own files. `systemfont_arial` and the rest are found among
//! this machine's fonts, as themselves or a look-alike with the same widths, and otherwise
//! stood in for by one of Wallpaper Engine's own. Characters none of a layer's fonts has come
//! from a wide font, Chinese, Japanese and Korean above all, when the machine has one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ab_glyph::{FontArc, FontVec};

use crate::Error;

/// Font files larger than this aren't read.
pub(crate) const FONT_BYTES: usize = 64 * 1024 * 1024;

const SANS: &str = "fonts/NotoSans-Regular.ttf";
const MONO: &str = "fonts/RobotoMono-Regular.ttf";

/// Wide fonts, best first, by file name.
const WIDE: &[&str] = &[
    "notosanscjk-regular.ttc",
    "notosanscjksc-regular.otf",
    "notosanssc-regular.otf",
    "notosanssc-regular.ttf",
    "sourcehansans-regular.ttc",
    "wqy-microhei.ttc",
    "wqy-zenhei.ttc",
    "droidsansfallbackfull.ttf",
    "droidsansfallback.ttf",
    "arial unicode.ttf",
    "hiragino sans gb.ttc",
];

pub(crate) fn parse(bytes: Vec<u8>) -> Result<FontArc, Error> {
    FontVec::try_from_vec_and_index(bytes, 0)
        .map(FontArc::new)
        .map_err(|_| Error::Format("not a font Wallpaper Engine's text could use".into()))
}

/// A system font by the name after `systemfont_`, and its size in bytes; `stock` reads one of
/// Wallpaper Engine's own fonts, which stands in when the machine has nothing like it.
pub(crate) fn system(
    name: &str,
    stock: impl FnOnce(&str) -> Option<Vec<u8>>,
) -> Result<(FontArc, usize), Error> {
    let (files, stand_in) = look_alikes(name);
    let own = format!("{name}.ttf");
    let found = files
        .iter()
        .copied()
        .chain([own.as_str()])
        .find_map(|file| read(installed().get(file)?).ok());
    match found {
        Some(font) => Ok(font),
        None => match stock(stand_in) {
            Some(bytes) => {
                let size = bytes.len();
                Ok((parse(bytes)?, size))
            }
            None => Err(Error::Format(format!(
                "the system font {name} isn't on this machine"
            ))),
        },
    }
}

/// Font files a system font is in, or one with its widths, and Wallpaper Engine's font that
/// stands in for it.
fn look_alikes(name: &str) -> (&'static [&'static str], &'static str) {
    match name {
        "arial" => (
            &[
                "arial.ttf",
                "liberationsans-regular.ttf",
                "arimo-regular.ttf",
            ],
            SANS,
        ),
        "consolas" => (
            &[
                "consola.ttf",
                "consolas.ttf",
                "liberationmono-regular.ttf",
                "cousine-regular.ttf",
                "dejavusansmono.ttf",
            ],
            MONO,
        ),
        "cambria" => (&["cambria.ttc", "cambria.ttf", "caladea-regular.ttf"], SANS),
        "comicsans" => (&["comic.ttf", "comic sans ms.ttf"], SANS),
        "segoe" => (
            &["segoeui.ttf", "segoe ui.ttf", "selawik-regular.ttf"],
            SANS,
        ),
        "verdana" => (&["verdana.ttf", "dejavusans.ttf"], SANS),
        "timesnewroman" => (
            &[
                "times.ttf",
                "times new roman.ttf",
                "liberationserif-regular.ttf",
                "tinos-regular.ttf",
            ],
            SANS,
        ),
        _ => (&[], SANS),
    }
}

/// The machine's wide font, read the first time a layer needs it.
pub(super) fn fallback() -> Option<&'static FontArc> {
    static FONT: OnceLock<Option<FontArc>> = OnceLock::new();
    FONT.get_or_init(|| {
        WIDE.iter()
            .find_map(|file| read(installed().get(*file)?).ok())
            .map(|(font, _)| font)
    })
    .as_ref()
}

/// Looks for the machine's fonts away from the thread that draws, so a script's first
/// character no layer font has costs no more than reading the wide font.
pub(crate) fn warm() {
    std::thread::spawn(|| {
        installed();
    });
}

fn read(path: &Path) -> Result<(FontArc, usize), Error> {
    let io = |error| Error::Io(path.display().to_string(), error);
    if std::fs::metadata(path).map_err(io)?.len() > FONT_BYTES as u64 {
        return Err(Error::Format("a font file too large to read".into()));
    }
    let bytes = std::fs::read(path).map_err(io)?;
    let size = bytes.len();
    Ok((parse(bytes)?, size))
}

/// The machine's font files by lower-case file name, looked for once.
fn installed() -> &'static HashMap<String, PathBuf> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut roots: Vec<PathBuf> = [
            "/usr/share/fonts",
            "/usr/local/share/fonts",
            "/Library/Fonts",
            "/System/Library/Fonts",
        ]
        .map(PathBuf::from)
        .into();
        if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
            roots.push(PathBuf::from(data).join("fonts"));
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            roots.extend(
                [".local/share/fonts", ".fonts", "Library/Fonts"].map(|dir| home.join(dir)),
            );
        }
        let mut index = HashMap::new();
        let mut budget = 50_000;
        for root in roots {
            walk(&root, 6, &mut budget, &mut index);
        }
        index
    })
}

fn walk(dir: &Path, depth: u32, budget: &mut usize, index: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => {
                if depth > 0 {
                    walk(&path, depth - 1, budget, index);
                }
            }
            Ok(_) => {
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    index.entry(name.to_lowercase()).or_insert(path);
                }
            }
            Err(_) => {}
        }
    }
}
