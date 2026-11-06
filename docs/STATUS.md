# Where things stand

Handoff note, 2026-11-06, at `e96dadd`. Development moves from the Mac to the Linux machine
that runs apolline-wall. [PLAN.md](PLAN.md) says what each feature does and which guesses it
rests on; [NOTES.md](NOTES.md) holds the limits and rules. This file covers how far it got,
what was never checked, and what to take next.

## How far it got

Of the 81 scenes in the local Workshop set (`~/we`), 55 prepare natively, and all 55 draw: every
one renders a frame without a shader failing to compile. These were added in the last stretch:

- Puppet warp: `animationlayers` and the `.mdl` format. The format was worked out from the
  scenes' own files.
- Layer colour blend modes (32).
- Bloom driven by a script.
- Material instances on layers.
- Child particle systems:
  - following each parent particle (`eventfollow`);
  - bursting where a particle is born or dies (`eventspawn`, `eventdeath`).
- Rope trails, and a sprite and a trail drawn from one system.
- Particle systems tilted out of the scene's plane.
- HDR scenes without bloom.
- Composed effect passes.
- The HLSL-to-GLSL conversion pass (`src/shader/dialect.rs`).
- A 1 GiB GPU budget.

## What was never checked

- **Real Linux GPUs.** Every shot and check so far ran on a Mac through desktop OpenGL 3.3.
  `tools/gles-check.sh` and `tools/render-all.sh` are the first things to run on the new
  machine. Under GLES, compilers are stricter about implicit int↔float conversions:
  - stock shaders that pass on the Mac may fail there;
  - `src/shader/dialect.rs` already spells most of those conversions out, but only where it can
    work out the types.
- **Audio.** apolline-wall never feeds the spectrum, so audio-reactive scenes run as if in silence:
  - bars and rings stay empty;
  - emitters and turbulence that listen stay off.

  It needs PipeWire capture into a 64-bin FFT per channel, given to `Renderer` the way `view`
  gives it.
- **Time.** `examples/view --time T` catches particle simulations up by at most 60 steps (one
  second). A shot at `--time 5` therefore shows particles as they'd be about one second in. Real
  playback advances frame by frame and isn't affected. To check particles at a later moment,
  raise `CATCH_UP` in `src/particles/simulation.rs` locally, and don't commit it.
- **Puppet `.mdl` files.**
  - Only each file's first animation is read: what follows an animation differs between versions.
  - Version 23 skeletons (`MDLS0004`) are refused.
  - Morph data after an animation's frames is ignored.

## What to take next

Cheapest first. The scene IDs are folders under `~/we/steamapps/workshop/content/431960`.

1. **Timelines on particle instance overrides** (`instanceoverride.alpha` with an `animation`).
   This is the only thing blocking 2998757800 and 3233141951. Scripts on overrides already work
   through `Target::Override`; timelines need the same target in `scene::animations` and the
   check.
2. **HDR bloom** (`general.hdr` with `bloom`). It is the only thing blocking 2468489223 and
   2784674898, and part of what blocks 2902406982 and 3299228616. Its settings are
   `bloomhdrstrength`, `bloomhdrthreshold`, `bloomhdrscatter`, `bloomhdrfeather` and
   `bloomhdriterations`; it looks like a chain of halving pictures. No stock material
   describes it, so work it out from the editor and the previews.
3. **Perspective layers** (`perspective: true` on an image). It is the only thing blocking
   2983846453, and part of what blocks 3113287126 and 3521337568.
4. **Particle materials with refraction** (`REFRACT` in `genericparticle`). They need a copy of
   the frame under the particles, which the bloom and blend-mode code already make
   (`compose::Frame`). Part of what blocks 2964778792, 2981249186, 1444077782 and 2370927443.
5. **Sprite renderer `flags` and `axis`**: the meaning isn't known. Part of what blocks
   2964778792 and 1444077782.
6. **`vortex`** (2419444134). Its defaults aren't documented anywhere reachable
   (docs.wallpaperengine.io is a single-page app that doesn't fetch), so don't guess.
7. **`thisScene.createLayer`** (3290320699): a script that makes layers at run time.
8. **The rest**, each its own project:
   - video textures (`.tex` with an MP4 inside, 3122339805);
   - 3D cameras and 3D layer rotation (3292361861, 3509243656, 3521337568);
   - `depthparallax` (3118949804);
   - scripts on text `pointsize` and particle `colorn` (2938612768);
   - user textures on effects (2370927443, 2388299037);
   - the decoded-texture budget (512 MiB on the CPU side, a separate budget from the GPU one;
     it blocks the Lain scene 2594686226 and 2388299037). Raising it is Danil's call.

## Tools

- `cargo run --release --example survey -- <workshop> <assets>` — how many prepare natively.
- `cargo run --release --example blockers -- <workshop> <assets>` — every scene that doesn't
  prepare, with its reasons.
- `tools/render-all.sh <workshop> <assets>` — draws one frame of every scene that prepares, and
  fails on any that won't draw. Run it after any change to shader preparation.
- `./target/release/examples/view [--time T] [--shot-size WxH] --shot out.png <project> <assets>`
  — one frame to compare with the project's `preview.*`.
- `python3 -I tools/pkg.py scene.pkg [file]` — lists a package or prints one of its files.
- `bash tools/gles-check.sh` — the GL checks against `tools/demo-scene.py`.

## Rules kept so far

- **Clean room.**
  - Never read or copy linux-wallpaperengine or any other GPL renderer.
  - Wallpaper Engine's own assets, docs, UI strings and the scenes' files may be read.
  - Reimplement the maths in WE's shader headers rather than copying them.
- **Refuse rather than guess.** Every guess that was made anyway is written down in PLAN.md
  next to its feature.
- **Pushing.** Commits go to `git@github.com:Pisosenie/apolline-scene.git` as one plain
  sentence, with no type prefix. apolline-wall pins this repository by commit in
  `packaging/arch/PKGBUILD`; bump the pin after each push here.
