<h1 align="center">apolline-scene</h1>

<p align="center"><b>Wallpaper Engine scenes, drawn natively on Linux, in Rust.</b><br>
A library that reads Workshop scene projects (packages, textures, materials, shaders, particles, effects and scripts) and draws them with OpenGL or GLES into any GL context it is handed. Written clean-room; nothing of Wallpaper Engine's is shipped.</p>

<p align="center">
<a href="https://github.com/Pisosenie/apolline-scene/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/Pisosenie/apolline-scene/actions/workflows/ci.yml/badge.svg"></a>
<img alt="License: MIT or Apache-2.0" src="https://img.shields.io/badge/license-MIT%20or%20Apache--2.0-5dd39e">
<img alt="Rust" src="https://img.shields.io/badge/Rust-2024%20edition-c084fc">
</p>

apolline-scene opens a Wallpaper Engine scene project the way the Workshop delivers it: `project.json`, `scene.pkg` (or a plain folder), `.tex` textures, materials and their shaders, effects, particle systems, puppet-warped models and SceneScript. It turns Wallpaper Engine's HLSL-flavoured shaders into GLSL, uploads what the scene needs and draws it through [glow](https://github.com/grovesNL/glow), on OpenGL ES 3 or desktop OpenGL 3.3, whichever context it is given. Scripts run in a [QuickJS](https://github.com/DelSkayn/rquickjs) runtime per renderer, with only the scene API written here: no filesystem, no network, a 64 MiB heap, 50 ms a frame and 10 more for promises, after which a script that keeps running is stopped.

It reads scenes from your own Wallpaper Engine install and Workshop downloads. A scene that uses something not done yet is refused up front with the reason, never drawn with parts quietly left out, so a host can hand it to another renderer instead.

## Who it is for

This crate is for developers. If you want Workshop scenes on your desktop, you want one of these:

- **[apolline-wall](https://github.com/Pisosenie/apolline-wall)**, the wallpaper daemon built on it. It draws scenes next to pictures, videos and shaders on every monitor, with the same crossfades, pause and fps limits, and hands scenes this library refuses to linux-wallpaperengine.
- **[Apolline](https://github.com/Pisosenie/apolline)**, the desktop shell that runs apolline-wall and shows its wallpapers.

## Where it is

On an Arch machine with an NVIDIA RTX 4070 Ti (proprietary driver, desktop GL and GLES through EGL), a real Workshop library of 41 items has 27 scenes that prepare natively. All 27 draw, and a frame of each matches the project's own preview. In apolline-wall the scenes ran on two monitors through niri, a rotated one included. What that run found and fixed:

- Stock `pulse` and the downsample effects name a variable `sample`, a reserved word in GLSL ES, so strict drivers wouldn't compile 9 of the 27. Reserved words in Workshop shaders are now renamed after the dialect pass.
- Turbulence noise was most of the CPU in particle-heavy scenes. It now walks its lattice once for its three noises: one such scene went from 35 % to 22 % of a core at 30 fps.

On the larger 81-scene set it was developed against on macOS, 55 prepare natively and all 55 draw. Mesa (AMD and Intel) hasn't been tried on real scenes yet, only on the synthetic ones in `tools/gles-check.sh`. [`docs/STATUS.md`](docs/STATUS.md) has the full picture and what to take next.

## What it draws

| Supported | Not yet |
| --- | --- |
| `scene.pkg` packages, plain project folders and Wallpaper Engine's `assets` | Video textures (`.tex` with an MP4 inside) |
| `.tex` textures: DXT1/3/5, RGBA, RG, R, LZ4-compressed, embedded PNG/JPEG, sprite sheets | Perspective image layers |
| Image layers placed, scaled, turned, tinted and blended, 32 colour blend modes | 3D cameras and 3D layer rotation, `depthparallax` |
| Camera parallax following the pointer, camera shake | HDR bloom (`general.hdr` with `bloom`); HDR scenes without bloom do draw |
| Materials with vertex and fragment shaders, includes, combos and `#if` conditions | Particle materials with refraction |
| Effects, multi-pass and composed, with the pointer, time of day and audio as inputs | The `vortex` operator, sprite renderer `flags` and `axis` |
| Bloom, also driven by a script | Ropes through a system's particles |
| Particles: emitters, initializers, operators, turbulence, control points, children (`eventfollow`, `eventspawn`, `eventdeath`), rope trails, systems tilted out of the plane | Timelines on particle instance overrides |
| Puppet warp (`.mdl`), partly: each file's first animation only, version 23 skeletons refused | `thisScene.createLayer` and scripts on text `pointsize` or particle `colorn` |
| SceneScript on objects, effects and particle overrides | User textures on effects (album art and the like) |
| Timeline animations on objects and effect values | Audio in apolline-wall: the library takes a spectrum, the daemon doesn't feed one yet |
| Text layers in the project's font or a system font | Opaque text backgrounds and letter spacing |
| User properties: bool, slider, colour, combo and text | 3D scenes, lights and models (not planned) |

`Scene::missing` and the error from `Project::prepare` name what a scene uses that isn't drawn yet. [`docs/PLAN.md`](docs/PLAN.md) says how each feature works and which guesses it rests on.

## Trying it

You need Wallpaper Engine (bought on Steam, run through Proton or not) for its `assets` folder, and some Workshop scenes. Subscribed items are folders named by their ID under `steamapps/workshop/content/431960` in each Steam library, for example `~/.local/share/Steam/steamapps/workshop/content/431960`; the assets are in `steamapps/common/wallpaper_engine/assets` of the library Wallpaper Engine is installed in.

```sh
ws=~/.local/share/Steam/steamapps/workshop/content/431960
assets=~/.local/share/Steam/steamapps/common/wallpaper_engine/assets

# A scene in a window; the pointer drives the parallax. Layers it can't draw are left out and named.
cargo run --release --example view -- $ws/1234567890 $assets

# One frame to a PNG, no window. Refuses a scene it can't draw whole unless --partial is given.
cargo run --release --example view -- --shot frame.png --shot-size 1920x1080 --time 2 $ws/1234567890 $assets

# Which scenes in a library prepare natively, and why the rest don't.
cargo run --release --example blockers -- $ws $assets
cargo run --release --example survey -- $ws $assets > survey.md

# One small frame of every scene that prepares; fails on any shader that doesn't compile.
tools/render-all.sh $ws $assets
```

`view` also takes `--properties values.json` (a JSON object of user property overrides) and `--stats`. `--time` catches particles up by at most one second, so a shot at `--time 5` shows them about one second in; playback in a window isn't affected. `python3 -I tools/pkg.py scene.pkg [file]` lists a package or prints one file from it, and `python3 tools/demo-scene.py <folder>` writes a small made-up scene to try things on without any Workshop content.

## Using it as a library

It isn't on crates.io; depend on it from git:

```toml
[dependencies]
apolline-scene = { git = "https://github.com/Pisosenie/apolline-scene" }
glow = "0.16"
```

A host opens a project, applies saved property edits, prepares the scene (which refuses what isn't supported before you replace the current wallpaper), builds a renderer in its GL context and drives it with its own clock:

```rust
use apolline_scene::{Project, Renderer, render::Fit};

let mut project = Project::open(folder, Some(assets))?;
project.set_properties(&saved_edits)?;           // apolline_scene::properties::Values
let prepared = project.prepare_pointed()?;       // prepare() if the host never moves the pointer

// With `gl` (a glow::Context) current on this thread:
let mut renderer = unsafe { Renderer::new(&gl, &prepared.scene, &prepared.layers)? };
if let Some(stage) = prepared.stage()? {
    renderer.script(stage);                      // the scene's scripts and timelines
}
let needs = renderer.needs();                    // pointer, daytime, audio: wire only these
let (width, height) = renderer.size();           // the scene's own size, for picking an aspect

// Each frame:
renderer.follow(pointer, delta);                 // -1..1 across the target, y up
unsafe {
    renderer.animate(&gl, time, delta);
    renderer.draw_timed(&gl, (w, h), Fit::Cover, time, delta);
}

// Before the GL context goes away:
unsafe { renderer.delete(&gl) };
```

`set_daytime` and `set_spectrum` (64 bins per channel) feed the other inputs when `needs` asks for them; `animated()` says whether the scene changes over time at all, so a still one can be drawn once. `Fit::Contain` letterboxes instead of cropping. [`docs/NOTES.md`](docs/NOTES.md) has the limits a host should know: texture and package size caps, the 1 GiB GPU budget, how scripts are bounded and when hidden layers stay loaded.

## Building and testing

A current stable Rust (the crate is edition 2024, so 1.85 at the very least). CI runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` on Ubuntu and macOS. The scripts in `tools/check-*.py` and `tools/gles-check.sh` are GPU regression checks that need a graphical session (or a headless sway on Linux).

The code is written clean-room: linux-wallpaperengine and other GPL renderers are never read. Formats are worked out from the scenes' own files and Wallpaper Engine's public documentation, and behaviour from comparing frames against the projects' previews.

## License

MIT or Apache-2.0, at your option.
