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

## What the first survey showed

60 of the most subscribed scenes and what they pull in (81 scene projects), surveyed
2026-11-06 with `tools/survey-set.py` and `examples/survey.rs`. Eight drew natively whole and
match their authors' previews; with scripts and text layers drawn, nine do, and with sprite
sheets eleven. What holds the
rest, by scenes:

- SceneScript, 32: mostly scripted origin, scale and alpha, and clocks in text layers. Scripts
  on objects' place and look, on text layers' words and on effects run now (below); what keeps
  those scenes out is mostly particle overrides (8), bloom and property conditions, and in four
  scenes
  `thisScene.createLayer`/`destroyLayer` and `getTransformMatrix`. None of the 15 scenes with
  effect scripts was held by them alone; the survey's "a few steps away" table shows what
  each near scene still lacks.
- Particle system `flags`, 31, and child systems, sprite sheets, ropes and trails, 10 each.
- Render targets (`_rt_FullFrameBuffer` and friends), 23: blend modes, composition layers.
- Text layers, 29 (`unknown-object`), drawn since (below). Groups, `parent` links and solid
  colour layers are drawn too: a child's origin is in its parent's units, turned and scaled
  with it, and a hidden parent hides it unless it disables propagation.
- Named targets in effects (`fbos`: godrays, blur, shine), 10. An older effect format marks a
  pass `compose` instead, drawn into a picture of its own for the next pass, as every pass
  here is; its vertex shader may put its quad in place without the matrix, as passes into
  buffers do, which is drawn the same way into the picture.
- The scene's bloom, 13: drawn over each finished frame as Wallpaper Engine's stock bloom
  materials lay it out (`downsample_quarter_bloom`, `downsample_eighth_blur_v`,
  `blur_h_bloom`, `combine`): a quarter size picture of what's above the threshold by its
  brightest channel, pushed away from grey and scaled by strength and tint, blurred down and
  across at an eighth over six steps of eight screen pixels each way, and added back. Our
  own shaders do the same sums. HDR bloom (`hdr` with `bloom`) is still refused; HDR without
  it is drawn as usual, which cuts what's brighter than white at each layer rather than at the
  end: the same, but where a translucent layer covers a pass that went past white. A script may drive
  `bloomstrength` (audio-reactive bloom); it starts from the scene's strength and sees no
  layer.
- Colour blend modes (`colorBlendMode`), 7: a picture layer is drawn into a target of its own
  and laid on by our program for its mode, which reads a copy of the frame under it where each
  pixel lands and mixes the two the way image editors do, in Wallpaper Engine's numbering
  (darken 1 through tint 30, then add-by-opacity 31 and A + AB 32). Most modes mix by the
  layer's alpha; min 5 and max 10 don't. Tint (30) checked against a preview's brightness to
  within 0.005. Text layers that blend are still refused.
- Puppet warp (`animationlayers`, a model's `puppet`), 12: the `.mdl` file's layout was worked
  out from the scenes' own files. Its mesh (`MDLV` 13, 16, 19, 21, 23) is corners in the
  picture's pixels from its middle, y up, each with up to four bones and weights and where in
  the picture it shows; then 16-bit triangles. Which attributes a corner has is in the mesh's
  flags (position 1, normal 2, tangent 4, picture 8, bones 0x01800000). The skeleton (`MDLS`)
  is each bone's name, parent and 4×4 rest matrix from its parent, then an editor's note, and
  then, from version 2, what the editor draws bones with, skipped by the section's end. An
  animation (`MDLA`) has an id, a name, `loop`, `mirror` or `single`, frames a second, a
  length, and per bone a frame more than its length, each a position, Euler angles in
  radians (only z read) and a scale. What follows each animation differs between versions,
  so only a file's first is read; a layer that plays another plays nothing. Corners are bent
  on the CPU each frame: each bone's pose is its layers' frames over its rest pose (mixed by
  `blend`, or added from rest when `additive`, at `rate`), placed through its parents, and
  each corner follows its bones by weight. The mesh is drawn in place of the layer's quad,
  showing the layer's picture after its effects, which work on the flat picture. Version 23
  skeletons are refused.
- Camera zoom (`general.zoom`), 3: drawn as the whole scene scaled about its middle, nearer
  above 1. One scene's preview matches it unzoomed, so which way it goes is a guess from a
  scene that animates it from 3 down to 1 as an opening; animated zooms aren't
  played yet.
- Big scenes over the 512 MiB texture budget, since DXT textures are decoded to RGBA.

Guesses it confirmed: pictures carry the clamp flag; particle key names; `VERSION` being the
only combo on most plain image materials; a random colour (`colorrandom`) picked at one point
between its ends, not a channel at a time (a stock snow from white to grey that came out in
every colour shows white and grey in its preview).

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

**C. Animated textures**, done for sprite sheets: an animated texture's `TEXS` section, read
from the 23 in the survey, lists its frames, each an image, a time and a place: the corner of
the box it fills and its top and left sides, which run backwards for a frame turned in the
sheet (v1 in whole pixels, v2 and v3 in floats, v3 with the frames' size in front). A stock
image material with `SPRITESHEET` plays them in turn, each for its own time, looping; the
layer is a frame's size, and a sheet of several images switches between them. Particles with
animated sprites are still refused.

**D. Particles**: a deterministic simulation core (emitters, initializers, operators as the
documentation names them, seeded), then parsing once the survey gives the JSON keys, then
one instanced draw per system; control points, children, trails and audio after. Static
children are drawn: each is one more system of the same object, let out once and drawn after
its parent, its origin in the parent's units and turned with it, three generations deep at
most. A child following particles (`eventfollow`) runs a twin of its parent's simulation (the
same file and seed, so the same particles) and lets its own out from each of the twin's: a
burst when one is born, its rate while it lives, `origin` away. Without the worldspace flag its
particles move with the one they came from and go when it dies (a shooting star's glow); with
it they stay where they were let out (a firefly's trail). The child's `maxcount` bounds all of
them together, and its own `starttime` gives way to its parent's. Children let out when
particles are born or die (`eventspawn`, `eventdeath`) are still refused. A rope trail
(`ropetrail`) is drawn as one strip from its particle back along its velocity for `length`
seconds, or its age if younger, its picture running end to end a width past each: the path it
came along, straightened. A system may list two renderers, a sprite and a trail behind it,
and each particle is drawn once by each. Ropes through a system's particles (`rope`), and rope trails that fade
or bend in segments (`fadealpha`, `segments`, `subdivision`), are still refused. Turbulence is drawn both ways the documentation gives it: a
turbulent start (`turbulentvelocityrandom`) turns its forward direction by `offset` radians
(stock leaves fall with 3) and by noise over when each particle is born, up to `scale` half
turns; the `turbulence` operator pushes each particle every second by noise over its place
and the time, at a speed of its own, along the axes its mask lets through. Their defaults
aren't written anywhere public, so those that no file sets are guesses (speeds 100–250 and
500–1000, operator scale 0.01, mask "1 1 0" since 3D examples spell out "1 1 1"); audio
response is refused. Control points are read too: each sits at its offset from the system's origin
or, with flag 1 (`locktopointer`), on the pointer, which the renderer maps back from the
screen through the scene's placement into the system's own units. `controlpointattract`
speeds particles within its threshold towards one, or away for a negative scale, so
cursor-avoiding dust and smoke work; a system that does makes the scene need the pointer.
Control points with other flags (children's, 4 and 16) refuse the operators that use them.
Systems may hold 50 000 particles and ask for any rate up to 100 000 a second. Scripts on
instance overrides (`count`, `rate`, `lifetime`, `size`, `alpha`, `speed`) run like those on
effect values: each is a driven value, and what it returns scales the running system by its
share of the value it started from, which the system was made with; a scripted count gets
room for the most particles. Emitters with an audio response (mode 3, both channels) let
out as much of their rate as the music is loud over their bands between their bounds, none
in silence; one-channel modes, and audio responses anywhere else, are refused. System flags
read as worldspace 1, sprite sheet frames without blending 2 (set with random frames in
stock embers), perspective 4, and colour, count, lifetime, size and speed overrides left out
8 to 128 (the stock lightning spawner's 248 is all five). One-channel particle sprites are
white shapes with the channel as alpha, as the stock particle shader converts them; they
were drawn as opaque grey squares before. A shader's own uniforms declared after an include
go first, since stock headers such as `common_blur.h` read them.

**F. SceneScript**, done for objects: each `{script, scriptproperties, value}` on an image,
particle system or group's `origin`, `scale`, `angles`, `color`, `alpha`, `brightness` or
`visible` is an ES module run in QuickJS (64 MiB, 50 ms a frame). `update(value)` returns the
new value; `thisLayer`, `thisScene.getLayer`, `engine` (clock, time of day, canvas, timers,
audio buffers, user properties), `shared`, `localStorage`, `input` (the cursor at rest),
`createScriptProperties`, `Vec2/3/4` and `WEMath`/`WEColor`/`WEVector` are written here from the
documented API. Angles are degrees to scripts. Each frame the objects' own values are read
back and composed through their parents, so a scripted group carries its children, and
hidden layers stay loaded for scripts to show. Cursor and media events never come. A script
that throws or asks for what isn't here refuses the scene with the member it asked for; one
that throws later just stops. Next: particle overrides (emitter rates per frame), then
`getTransformMatrix`.

Scripts on effects run too: on an effect's `visible`, where `thisObject` is the effect, and on
the values its passes give their shaders (`update(value)` gets a number, a vector or a flag
and returns the new one, which becomes that uniform). `thisLayer.getEffect(place or name)` and
`getEffectCount()` reach any of a layer's effects. A hidden effect stays loaded when a script
could show it: one on its `visible`, or any script that looks effects up. One that can't be
drawn doesn't refuse the scene; a script that shows it fails with why, which refuses the scene
on its first frame and stops just that script later. Media events that would show such
effects (album art through `usertextures`) never come, so those scenes aren't held by them. A
chain draws again only when a script changes what it draws.

**Text layers**, done: words in the project's font, or a system font (`systemfont_arial` and
the rest found on the machine or a look-alike with its widths, else Wallpaper Engine's own Noto
Sans or Roboto Mono), drawn into a texture and placed like a picture, so effects and parents
work on them. Sizes follow Wallpaper Engine's, checked against the sizes its editor saved in
real scenes: a point is 1/300 inch, a line the font's ascent, descent and gap, padding clear
room outside the words, and the alignment the side of the words the origin is on. Lines wrap
at `maxwidth`, stop at `maxrows`, with an ellipsis if asked; characters the font lacks come from
the machine's wide font. A script on `text`, or `thisLayer.text`, changes the words; the
texture and its effect targets are drawn again at the new size. Opaque backgrounds, letter
spacing and block alignment are refused; `anchor` (keeping a layer by a screen edge on
screens shaped unlike the scene) is ignored.

**E. Timeline animations**, done: an `animation` on an object's `origin`, `scale`, `angles`,
`color`, `alpha` or `brightness`, or on an effect's value, plays each frame before the scripts
run, so a script on the same value gets what the timeline gives. Each component has its own
keys; between two keys the value follows a cubic with the slopes the key handles give: flat
for the editor's default handles (easing in and out), through the neighbouring keys for
automatic (`magic`) ones, straight at the neighbour for a disabled one, and the handle's rise
over its run for one dragged by hand. That last reading is inferred from what the editor saved
for automatic handles, which fit it on their longer side; no real scene's hand-dragged handles
could be checked against Wallpaper Engine. Timelines loop, play once and hold, or mirror; a
loop with `wraploop` runs from its last key back to its first; `relative` adds to the value;
`startpaused` waits at frame 0. `getAnimation(name)` on a layer or effect plays, pauses, stops
and sets frames. Animation events (`events`) aren't sent, and `parent`/`children` links don't
tie playback together beyond all starting at once. Timelines on visibility, text, particle
overrides and the camera, turns out of the scene's plane, an effect material's own values and
more than 4096 in a scene are still refused.

**G. puppet warp**: once its storage is known from real files. 3D scenes, lights and models
are not planned.

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
- Effects (B3–B5): each shown instance names an `effect.json` whose passes name one-pass
  materials; the instance's passes override combos, values and textures. A layer with effects
  is drawn into a target of its texture's size, each pass over the last back and forth
  between two targets, and the result placed like a plain layer; a chain without time is
  drawn once. Named targets (`fbos`, `target`, `bind`), passes that blend and animated values
  are refused by name, so the survey can count them. `--effect-demo` draws exactly what the
  plain demo draws, through two inversions and a tint.
- Engine inputs (B6): `g_PointerPosition` and `g_ParallaxPosition` (0 to 1 from the top
  left, a guess), `g_Daytime` from `Renderer::set_daytime`, and the audio spectrum in 16,
  32 and 64 bins a side from `Renderer::set_spectrum`, silent until a host gives one.
  `Renderer::needs` says which a scene uses. `prepare` refuses pointer readers as it does
  pointer parallax, for hosts that don't move the pointer; `prepare_pointed` takes them for
  hosts that do, through `Renderer::follow`, and apolline-wall does: on Wayland from the seat
  while the pointer is over the desktop, on X11 by asking each frame, on GNOME (whose window
  lets clicks through) left at the centre. Effects that follow the pointer, like xray, get
  where their layer is on screen turned around: `g_EffectTextureProjectionMatrixInverse` from
  the screen to the layer's picture, -1 to 1 with y up, and
  `g_ModelViewProjectionMatrixInverse` to the pass's own units.
- Particles (D1–D3): box and sphere emitters; lifetime, size, alpha, colour, velocity,
  rotation and spin initializers; movement, angular movement, alpha fade, size, alpha and
  colour change operators; one sprite renderer with the plain `genericparticle` material.
  Simulated in sixtieths of a second from the host's scene time with a fixed seed, so the
  same moment always looks the same; `starttime` warms the system up. Drawn as instanced
  sprites in scene order. Everything else — control points, turbulence, ropes, sprite
  sheets, instance overrides — is refused by name. Key names and units (colours in 0–255,
  life shares for fades) are guesses for the survey to confirm.
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
- A material or effect value the selected shader code doesn't read refuses the material.
  Scene files may keep values for every option of an effect; if the survey shows that, such
  values should be let through.
- A texture repeats unless its flags say to clamp, and one padded to a larger size clamps
  anyway; effect targets clamp. Which flags real pictures and noise textures carry is for the
  survey to show.
- Without a vertex shader of its own, a `vec4 v_TexCoord` gets zeros in `zw`; `g_Color` and `g_Alpha` are always 1 with the tint
  applied after, and `g_TexelSize` follows the framebuffer rather than the layer.

The shader-language aliases (`texSample2D`, `frac`, `saturate` and the rest) are named after
what Wallpaper Engine's own shipped shaders use, which any renderer of them has to accept.
