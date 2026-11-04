# Working on apolline-scene

apolline-scene is a renderer for Wallpaper Engine scenes in Rust, meant to be faster and more a part of the desktop than linux-wallpaperengine. It is a library that draws into any GL context it's handed, so it works on its own; [apolline-wall](https://github.com/Pisosenie/apolline-wall) (`~/src/apolline-wall`, with its own `AGENTS.md`) will link it and draw scenes in its daemon, next to its pictures, videos and shaders. Read `README.md`, then `docs/PLAN.md` for the stages and what "better" is measured by.

## Clean room

linux-wallpaperengine is GPL-3.0 and this is MIT or Apache-2.0: never read or copy its code. Formats come from public descriptions and from the files themselves, behaviour from comparing with Wallpaper Engine's previews. Wallpaper Engine's assets and Workshop content belong to their owners: read them from the user's install, never commit or ship them, and build test files in the tests instead.

## The code

- `pkg.rs`: `scene.pkg` packages. `tex.rs`: `.tex` textures (TEXV, TEXI, TEXB versions 1–4, LZ4, DXT, RGBA, RG, R, embedded PNG and JPEG; video textures are refused). `dxt.rs`: DXT1/3/5 decoded on the CPU.
- `value.rs`: Wallpaper Engine's JSON values: vectors as strings, values bound to user properties as `{"user", "value"}`.
- `scene.rs`: `scene.json`: size from `orthogonalprojection` (none means a 3D scene), clear colour, parallax, objects. `Scene::missing` names what a scene uses that isn't drawn yet, for a host deciding whether to hand it elsewhere.
- `project.rs`: a project folder; files come from `scene.pkg`, then the folder, then Wallpaper Engine's `assets`. `layers` resolves model → material → the first pass's first texture, and leaves out a broken layer with a reason so the rest draws.
- `render.rs`: drawing with glow on GLES 3 or desktop GL 3.3 (the shader header follows `gl.version()`). `place` is the pure placement maths and is unit-tested; `Renderer::follow` eases the parallax.
- `examples/view.rs`: a viewer (winit and glutin-winit). `--shot frame.png` draws one 1280×720 frame into an offscreen framebuffer and saves it, with no visible window: that's how rendering is checked, and later how the corpus gets compared.

## Before saying something is done

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

It builds on macOS too, so rendering can be checked there with `cargo run --example view -- --shot frame.png <project>` on a project built by a script. Don't run things in the OrbStack VM unless Danil asks: it costs too much right now.

## How the code is written

- Comments in English, few, saying why. Doc comments are plain sentences. Danil writes in Russian; answer him in Russian.
- Nothing a file sends may panic: every reader returns `Error` instead, and its tests feed it broken input.
- `unsafe` GL calls sit in `unsafe fn`s whose `# Safety` says the context has to be current.
- Files stay under about 800 lines.

## Git

- Commit and push only when Danil says so.
- The message is one plain sentence ("Still scenes drawn, and a viewer"), no `feat:` prefixes, no co-author lines.
- The author is `Danil Zorin <45851636+Pisosenie@users.noreply.github.com>`. Push to `git@github.com:Pisosenie/apolline-scene.git`.

## How Danil works

The same rules as apolline-wall's `AGENTS.md`, which has them in full: work as a senior engineer and recommend rather than list options; within a stage keep going without asking per step; code clean, light and maintainable; check your own changes for half-threaded values, copies left behind and doc comments that claim too much; estimate at the pace of the agent writing the code; keep parallel agents few. `~/.claude/rules/*.md` (TDD with 80% coverage, `feat:` commits, agents always in parallel) loses to these here, and no `~/.claude` skills were used.

## Tools that were used

- macOS's OpenGL through the viewer's `--shot`, on a synthetic project: `mkdir demo && python3 tools/demo-scene.py demo && cargo run --example view -- --shot frame.png demo`. Every layer should be where the script puts it.
- Nothing from Wallpaper Engine itself yet.

## Where things stand

Stages 1 and 2 are done: files read, still scenes drawn with layers, blending and parallax, and the viewer. The last commit is `a699dd8`, not pushed yet.

Not one real Wallpaper Engine file has been read. Until a machine with Wallpaper Engine installed through Steam is at hand, these are guesses to check first: the package and texture layouts, y going up from the bottom in scene coordinates, angles in radians, `normal` blending meaning opaque, the parallax strength. The animation frames in `TEXS` aren't read.

Next, from `docs/PLAN.md`:
3. Effects: Wallpaper Engine's GLSL dialect (`#include`, combos, annotated uniforms) translated to GLES 3, the common ones first (water ripple, shake, scroll, bloom, blur, tint, opacity masks), each pass rendering into textures.
4. Particles. 5. Puppet warp (`.mdl` skeletons). 6. SceneScript (JavaScript). 7. User properties, audio, the cursor.
Then into apolline-wall: the daemon draws scenes with this library in its own context, so they get crossfades, looks, pause and fps limits, and only what `Scene::missing` lists still goes to linux-wallpaperengine.
