# apolline-scene plan

A renderer for Wallpaper Engine scenes, as a Rust library that draws into a GLES context it
is handed. [apolline-wall](https://github.com/Pisosenie/apolline-wall) links it and draws
scenes in the same context, frame callbacks and crossfades as its pictures, videos and
shaders; until then it hands scenes to `linux-wallpaperengine` as a separate process.

## Better than linux-wallpaperengine, measured

"Better" is a number, not a feeling:
- **Correct:** the share of a fixed corpus that looks right. The corpus is the 200 most
  subscribed Workshop scenes, each checked against Wallpaper Engine's own preview. It's
  tracked per stage.
- **Cheap:** CPU, GPU and memory per scene, start time, and nothing at all while covered.
- **Part of the desktop:** crossfades, looks, pause and fps limits come with the host, not
  bolted on from outside.

## Clean room

`linux-wallpaperengine` is GPL-3.0 and this library is MIT or Apache-2.0, so its code is not
read or copied. Formats come from public descriptions and from reading the files themselves;
behaviour from comparing against Wallpaper Engine's previews. Wallpaper Engine's assets and
Workshop content stay their owners': they're read from the user's own install and never
shipped.

## Stages

1. **Files.** `scene.pkg` packages, `.tex` textures (DXT1/3/5, RGBA, RG, R, LZ4-compressed,
   embedded PNG/JPEG, animated frames), and a project read from a package or a plain folder.
2. **Still scenes.** `scene.json`: the camera, image layers with their origin, scale, angle,
   colour, alpha and blending, parallax. A viewer binary to look at a scene alone.
3. **Effects.** Wallpaper Engine's shaders: its GLSL dialect with `#include`, combos and
   annotated uniforms, translated to GLES 3. The common ones first: water ripple, shake,
   scroll, bloom, blur, tint, opacity masks.
4. **Particles.** Emitters, initializers, operators and renderers.
5. **Puppet warp.** `.mdl` models with skeletons and their animations.
6. **SceneScript.** The JavaScript in scenes, on an embedded engine.
7. **Properties, audio and the cursor.** User properties as controls in apolline-wall,
   audio-reactive values from a PipeWire capture, and the cursor.

A scene that uses something not done yet goes on falling back to `linux-wallpaperengine`.

## The road past linux-wallpaperengine

A scene draws natively only when all of it is supported, so the order follows joint coverage
of typical popular scenes. Public documentation (docs.wallpaperengine.io) gives what things
mean, not how the files store them; Wallpaper Engine's own developer has said multi-pass
effects and render targets aren't documented. So every format guess stays behind a refusal
until the survey (A3), run on a real install, confirms it. A wrong native picture is worse
than a lossless fallback.

**A. Ground truth and safety**
1. Split `render.rs` (layers, host state) and the project tests before they grow; the
   renderer saves and restores the host's framebuffer and viewport around a draw.
2. Refuse what isn't understood: an allowlist of keys per object kind and for `general`,
   values with keys beside `user`/`value` (animated or scripted), visible `general` flags;
   `Scene::missing` returns coded reasons the host and the survey can count.
3. `examples/survey.rs`: run over a Workshop folder and the assets, it reports the shapes of
   files (key paths, value types, counts, public vocabulary like effect and component names,
   texture header numbers) and the refusal reasons, never shader text or pixels, and a
   greedy list of what to support next.

**B. Effects**
1. Material values as uniforms instead of baked constants: one program per shader and combos,
   values set per draw, which also makes live property edits possible.
2. Material vertex shaders: `a_Position` and `a_TexCoord`, the documented matrices, varyings
   matched by name and type between the stages.
3. Effect instances and single-pass `effect.json`, parse-only, everything else refused.
4. A render-target chain per layer: the layer, each pass ping-ponged, then the composite;
   chains without time run once and are cached.
5. Effects reach apolline-wall; `Project::prepare` is the authority on what it refuses.
6. Engine inputs: pointer, daytime, parallax and audio spectrum (silence by default), and
   `Renderer::needs` so the host wires only what is used.
7. Multi-pass effects with named targets, once the survey shows their structure.
8. Scene-level targets (`_rt_FullFrameBuffer`, bloom), once previews can be compared.

**C. Animated textures**: the frame plumbing now, `TEXS` parsing once real headers are known.

**D. Particles**: a deterministic simulation core (emitters, initializers, operators as the
documentation names them, seeded), then parsing once the survey gives the JSON keys, then
one instanced draw per system; control points, children, trails and audio after.

**E. Timeline animations**, **F. text layers and SceneScript** (QuickJS with memory and time
limits), **G. puppet warp**: each once its storage is known from real files. 3D scenes,
lights and models are not planned.

**Measured.** With real files: the corpus of the 200 most subscribed scenes, each shot
natively and through linux-wallpaperengine as a black box, against the author's own preview;
native coverage, the share that looks right for each, and start time, CPU and GPU per frame,
memory, and nothing while covered. Now: `view --bench` on synthetic scenes (many layers,
static and animated chains, thousands of particles), recorded per commit.

## Now

Written and tested on files built in the tests and on macOS's desktop OpenGL; not one real
Wallpaper Engine file has been read yet.

- Stages 1 and 2: packages and textures with bounds on every size, still scenes with layers,
  blending and parallax, the viewer and its `--shot`.
- User properties: bool, slider, colour, combo and text, bound into the scene. A property
  described in a way it can't be edited stays read-only, and a saved edit that no longer fits
  gives way to the default, so neither keeps a scene from loading.
- The start of stage 3: one-pass fragment materials. Includes, combos and `#if` conditions
  with a bounded expression evaluator, numeric constants, up to eight texture inputs (R8 and
  RG8 too), optional samplers and default textures, `g_Time` and `g_Frametime`. Material
  values are uniforms set per layer, so one program serves every layer with the same shader
  and combos. Stock materials with `cullmode`, `depthtest` and `depthwrite`, and
  `genericimage2`, draw as plain layers.
- Material vertex shaders (B2): prepared with the fragment through the same includes, combos
  and values; `a_Position` is the layer's corner in its own units, centred, `a_TexCoord` its
  texture coordinate, `g_ModelViewProjectionMatrix` places it; any varyings, matched by name
  and type. Vertex shaders that sample or read other engine uniforms are refused.
- `examples/survey.rs` (A3): the shapes of a real install, for deciding what comes next.
- apolline-wall draws the scenes this covers with `Project::prepare` and flattens still ones
  once; everything else still goes to linux-wallpaperengine.

Guesses to check on real files first: y going up in scene coordinates and angles in radians,
`normal` blending meaning opaque, the parallax strength, and the frames of `TEXS` textures,
which aren't read.

Known gaps in the material path, from review:
- Sampler defaults follow the material's combos, but not combos that only turn on because a
  texture is bound.
- GLSL ES 3.00 is stricter than desktop GL 3.3 about implicit int-to-float conversion, which a
  real Workshop shader may trip over. The synthetic ones — plain layers, a fragment material, a
  mask, a time-driven wave, and `mul`, `lerp`, `fmod` (truncating, as HLSL's does) with an `out`
  helper — draw the same pixels on Mesa's GLES and on macOS's desktop GL (`tools/gles-check.sh`).
- `{"user": {"name", "condition"}}` bindings aren't followed; `Scene::missing` names them, so
  such scenes go to linux-wallpaperengine.
- Without a vertex shader of its own, a `vec4 v_TexCoord` gets zeros in `zw`; `g_Color` and `g_Alpha` are always 1 with the tint
  applied after, and `g_TexelSize` follows the framebuffer rather than the layer.

The shader-language aliases (`texSample2D`, `frac`, `saturate` and the rest) are named after
what Wallpaper Engine's own shipped shaders use, which any renderer of them has to accept.
