//! Reproducible mutation smoke test for readers; complements, rather than replaces, fuzzing.
//! `cargo run --release --example check_readers -- SECONDS [SEED] [FAILURE_FILE]`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use apolline_scene::{Scene, pkg::Package, shader::Fragment, tex::Texture};

struct Random(u64);

impl Random {
    fn next(&mut self) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 as usize
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let seconds = args
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30)
        .clamp(1, 3600);
    let seed = args
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0x6d61736b)
        .max(1);
    let failure = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reader-failure.bin"));
    let mut random = Random(seed);
    let mut texture = b"TEXV0005\0TEXI0001\0".to_vec();
    for value in [0u32, 0, 2, 2, 2, 2, 0] {
        texture.extend(value.to_le_bytes());
    }
    texture.extend(b"TEXB0002\0");
    for value in [1u32, 1, 2, 2, 0, 0, 16] {
        texture.extend(value.to_le_bytes());
    }
    texture.extend([128; 16]);
    let mut package = Vec::new();
    package.extend(8u32.to_le_bytes());
    package.extend(b"PKGV0021");
    package.extend(1u32.to_le_bytes());
    package.extend(10u32.to_le_bytes());
    package.extend(b"scene.json");
    package.extend(0u32.to_le_bytes());
    package.extend(2u32.to_le_bytes());
    package.extend(b"{}");
    let scene = br#"{"general":{"orthogonalprojection":{"width":1920,"height":1080}},"objects":[{"image":"model.json","visible":true,"origin":"0 0 0"}]}"#.to_vec();
    let shaders = [
        "// [COMBO] {\"combo\":\"MODE\",\"default\":1}\n#if MODE\nvarying vec2 v_TexCoord;\nuniform sampler2D g_Texture0;\nvoid main() { gl_FragColor = texSample2D(g_Texture0,v_TexCoord); }\n#else\nuniform float g_Time;\n#endif",
        "#ifndef HEADER\n#define HEADER\n#define VALUE 1 + 2\n#if VALUE * 3 == 7 && defined(GLSL)\nvoid main() { gl_FragColor = vec4(1.0); }\n#endif\n#endif",
        "#include \"fuzz.h\"\nvoid main() { gl_FragColor = vec4(1.0); }",
    ];
    let seeds = [package, texture, scene]
        .into_iter()
        .chain(shaders.map(|source| source.as_bytes().to_vec()))
        .collect::<Vec<_>>();
    let start = Instant::now();
    let mut cases = 0u64;
    let mut counts = [0u64; 4];
    while start.elapsed() < Duration::from_secs(seconds) {
        let choice = random.next() % seeds.len();
        let mut input = seeds[choice].clone();
        for _ in 0..1 + random.next() % 8 {
            if input.is_empty() {
                input.push(0);
            }
            let index = random.next() % input.len();
            match random.next() % 5 {
                0 => input[index] ^= 1 << (random.next() % 8),
                1 => input[index] = random.next() as u8,
                2 => input.truncate(index),
                3 => {
                    input.remove(index);
                }
                _ => input.insert(index, random.next() as u8),
            }
        }
        let kind = choice.min(3);
        let checked = std::panic::catch_unwind(|| match kind {
            0 => {
                let _ = Package::parse(input.clone());
            }
            1 => {
                let _ = Texture::parse(&input);
            }
            2 => {
                let _ = Scene::parse(&String::from_utf8_lossy(&input));
            }
            _ => {
                let _ = Fragment::prepare(
                    "fuzz.frag",
                    &Default::default(),
                    &Default::default(),
                    |name| {
                        Ok(match name {
                            "shaders/fuzz.frag" => Some(String::from_utf8_lossy(&input).into_owned()),
                            "shaders/fuzz.h" => Some("#ifndef FUZZ_HEADER\n#define FUZZ_HEADER\nfloat helper(float x) { return x; }\n#endif".into()),
                            _ => None,
                        })
                    },
                );
            }
        });
        if checked.is_err() {
            let saved = std::fs::write(&failure, &input);
            eprintln!(
                "reader panic: seed={seed}, case={cases}, kind={kind}, saved={saved:?}, path={}",
                failure.display()
            );
            std::process::exit(1);
        }
        counts[kind] += 1;
        cases += 1;
    }
    println!(
        "seed={seed} cases={cases} seconds={:.3} package={} texture={} scene={} shader={} panics=0",
        start.elapsed().as_secs_f64(),
        counts[0],
        counts[1],
        counts[2],
        counts[3]
    );
}
