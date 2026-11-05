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
  RG8 too), optional samplers and default textures, `g_Time` and `g_Frametime`, and one cached
  program per material. Stock materials with `cullmode`, `depthtest` and `depthwrite`, and
  `genericimage2`, draw as plain layers. A material's own vertex shader gives way to ours as
  long as its fragment wants only the layer's coordinates.
- apolline-wall draws the scenes this covers with `Project::prepare` and flattens still ones
  once; everything else still goes to linux-wallpaperengine.

Guesses to check on real files first: y going up in scene coordinates and angles in radians,
`normal` blending meaning opaque, the parallax strength, and the frames of `TEXS` textures,
which aren't read.

Known gaps in the material path, from review:
- Sampler defaults are looked up before `#if` branches are chosen, so an inactive branch's
  default can still refuse a material.
- GLSL ES 3.00 is stricter than desktop GL 3.3 about implicit int-to-float conversion, and
  nothing has run on GLES yet; `mul`, `lerp` and `fmod` have no aliases, and helpers with `out`
  parameters are refused.
- `{"user": {"name", "condition"}}` bindings keep their stored value instead of following the
  property.
- A `vec4 v_TexCoord` gets zeros in `zw`, `g_Color` and `g_Alpha` are always 1 with the tint
  applied after, and `g_TexelSize` follows the framebuffer rather than the layer.

The shader-language aliases (`texSample2D`, `frac`, `saturate` and the rest) are named after
what Wallpaper Engine's own shipped shaders use, which any renderer of them has to accept.
