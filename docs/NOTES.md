# Implementation notes

How the host-facing pieces behave in detail. `docs/PLAN.md` has the stages and the current state.

## Hosting still scenes

`Project::prepare_still()` returns a complete still scene or a reason to use another renderer.
`Project::prepare()` also accepts time-driven materials, effects, particles and scripts on
objects, while rejecting animated texture frames, pointer parallax, 3D and unsupported
materials. `Renderer::draw_fitted` supports cover and contain; `draw` keeps its cover default.
Repeated layers share decoded pixels through `Arc<Texture>` and one GPU upload per renderer.
Hidden layers load no resources. Decoded textures are limited to 256 MiB across frames and
mipmaps, with dimensions checked before decompression. GPU resources are released when
construction fails; the host still calls `Renderer::delete` before destroying its GL context.

A scene may retain at most 512 MiB of distinct decoded textures and 4096 image layers.
Shared textures count once. Packages are capped at 512 MiB, individual resources at 256 MiB
and JSON files at 8 MiB. Readers reject non-regular files and bound reads even if a file grows.
These are separate allocation limits, not a guarantee that total process RSS stays below 512 MiB.

## Scripts

`Prepared::stage()` loads the scene's scripts afresh for one renderer, which runs them through
`Renderer::script(stage)`; the host calls `Renderer::animate(time, delta)` before each
`draw_timed`, and `Renderer::needs` includes the time of day and audio when scripts read them.
Scripts that fail to load or throw in their first frame refuse the scene in `prepare`. Each
renderer has its own QuickJS runtime with a 64 MiB heap, a 1 MiB stack and 50 ms a frame; a
script cut short or throwing later stops, the rest go on. Scenes with scripts keep hidden
layers loaded, since a script may show them. `prepare_still` refuses them.

## User properties

`Project::properties()` describes the controls declared in `project.json` under
`general.properties`. `set_properties(&Values)` validates boolean, slider, color, combo and
text overrides; defaults apply to keys left out. Binding objects with `user` and `value`
are resolved in scene, model and material JSON before layers are prepared. Invalid edits
leave the previous values intact. Unknown property types remain visible in the schema,
but edits to them are refused; conditions and scripts are never evaluated.

The viewer accepts `--properties values.json`, a JSON object of key/value overrides. The demo
scene exposes sun visibility, opacity and tint; generate it with `tools/demo-scene.py demo
--still`. These bindings are tested against synthetic projects; real Wallpaper Engine metadata
and authored scenes still need corpus validation.

Property types and value representations follow the public [Wallpaper Engine user-property
documentation](https://docs.wallpaperengine.io/en/web/customization/properties.html).


## Preparation bounds and reuse

Within one layer preparation, a resolved model (including its material, texture and failure
result) is reused by all layers that reference it. Layer transforms remain independent and
textures are still shared through Arc. The cache is local to that preparation: editing files
or changing property values is observed on the next call, without a long-lived stale cache.
Models retain independent placement; their full material result (including blending, defaults
and errors) is reused by resource path during one preparation.

The native still path rejects animated texture flags and multiple image records while reading
headers, before allocating or decompressing their pixels. The general texture reader retains
its previous ability to decode multiple images; this is an early fallback decision for still
preparation, not native animation support.

Texture tags must be exactly eight ASCII bytes followed by NUL. Package version strings are
limited to 32 bytes, resource names/paths to 4096 bytes, and package filenames to 8 MiB in
total. Package strings must be UTF-8. These checks precede metadata copying and keep malformed
headers from creating huge diagnostic strings. They are reader policy limits, verified on
synthetic fixtures; compatibility with real author assets still needs corpus validation.


## Fragment materials

The first part of stage 3 supports a single fragment-only material pass on a 2D image layer.
`shader: "demo/filter"` loads `shaders/demo/filter.frag`; a companion custom `.vert` refuses
native preparation. Materials accept one to eight decoded RGBA, R8 or RG8 textures (including supported DXT and
embedded-image formats). Every required input slot must be bound. Custom fragments sample raw
channels: R8 gives (r,0,0,1), RG8 gives (r,g,0,1). Ordinary image layers keep their existing
greyscale / greyscale-plus-alpha conversion in the image shader. Both paths share one upload;
compact masks do not expand to RGBA. Supported fragments run directly at
the target framebuffer resolution, preserving procedural detail even for small input textures.
They share the layer's placement, clipping, parallax and blending. The actual layer tint and
alpha are applied once after the custom fragment; declared g_Color/g_Color4/g_Alpha are neutral
inside this pass. Layer alpha blends RGB over the scene background; the final framebuffer stays
opaque. In wall's native still path the whole scene is flattened once as before.

The supported source subset includes quoted relative/global shader includes, [COMBO] integer
or boolean defaults/overrides, legacy varying/gl_FragColor, common sampling/casting aliases,
and scalar/vector material constants from annotations or constantshadervalues. Bound project
properties are resolved before constants become GLSL literals. UV input is v_TexCoord as vec2,
vec3 or vec4; g_Texture0 through g_Texture7 and their Resolution uniforms are provided.
Unknown constants, unsupported dynamic uniforms, uniform arrays, unsupported texture metadata and unsupported pass
fields refuse preparation rather than receive invented values. Image effect arrays and
multi-pass materials are still unsupported.

A bounded conditional pass selects #if/#ifdef/#ifndef/#else/#elif/#endif branches before
uniform introspection. Inactive dynamic uniforms, alternate UV declarations and unused
sampler slots do not force fallback. The same selected source reaches the GPU. Integer and
boolean expressions support defined, comparisons, bitwise operations, arithmetic and short
circuiting. Object macros expand textually, preserving operator precedence; include guards
and #undef are respected. Conditions depending on GL_* or __* platform macros, function
macro values, octal/unsigned literals, continuations and overflowing arithmetic refuse this
subset. Explicit constants with no active matching uniform still refuse preparation.

Limits are 64 conditional levels, 256 macros, 1024 input/4096 expanded expression tokens,
16 macro-expansion levels and 65536 token-expansion steps per shader. Driver-provided
compatibility macros and engine uniforms cannot be aliased. Author #line directives are
refused; generated source IDs stay under host control. This is a supported subset rather
than a complete C or Wallpaper Engine preprocessor.

Includes are bounded to 16 levels, 64 source files and 256 directives. A file may contain up
to 256 KiB, combined input and expanded source up to 1 MiB each. Includes are expanded before
GLSL conditional evaluation, so a missing include in an inactive branch also refuses this
subset. Host macros cannot be redefined. There may be 64 custom materials/programs and 16 MiB
of prepared source per preparation/renderer. Unique GPU texture allocations have a separate
512 MiB logical budget; it is not a whole-process RSS or exact VRAM guarantee. Driver shader
compilation is not sandboxed or time-bounded by these source limits.

Programs and textures are reused across repeated layers; failed construction deletes resources
already created. Diagnostics name source IDs and cap driver logs. Preparation reports at most
64 detailed layer errors (bounded names/reasons) plus an omitted-count summary. The viewer's
`--shot` path refuses scenes with skipped resources or unsupported scene features instead of
producing a misleading partial reference image.

Try a synthetic desaturation material, including a local header, combo and bound amount:

```sh
python3 tools/demo-scene.py demo --still --fragment-demo
cargo run --example view -- --shot effect.png demo
# values.json may contain {"effect_amount":0} for an identity comparison
cargo run --example view -- --properties values.json --shot identity.png demo
```

A two-input opacity-mask fixture uses an R8 mask by default and shared texture budgets:

```sh
python3 tools/demo-scene.py masked --still --mask-demo
cargo run --example view -- --shot mask.png masked
# --mask-format rg or rgba selects another mask storage format
# mask_amount = 0 disables the mask; 1 applies its red channel to the layer alpha
```

On macOS GL, this fixture was rendered with vec2 and vec4 UVs; a zero-strength pass matched
the baseline pixels exactly, a full-strength pass produced grayscale background pixels, and
a procedural UV test retained detail at target resolution. Invalid GLSL was rejected without
a partial screenshot. These synthetic checks do not establish compatibility with real Workshop
materials or GLES driver behavior. The subset follows the public
[shader syntax](https://docs.wallpaperengine.io/en/scene/shader/syntax.html) and
[variables documentation](https://docs.wallpaperengine.io/en/scene/shader/variables.html);
no third-party renderer implementation or stock shader assets were copied.


`cargo run --example view -- --stats --shot frame.png demo` reports unique texture uploads and
logical uploaded mipmap bytes, excluding driver overhead. The GPU regression
`python3 tools/check-material-channels.py` checks R8/RG8 raw sampling alongside image conversion
and verifies shared uploads, in both layer orders. It needs a working graphical GL session and
cached Cargo dependencies; it is separate from the headless unit tests.


`python3 tools/check-shader-conditions.py` compares 64 conditional variants with the system
C preprocessor and checks their output colors on GL. It needs a C compiler and the same
graphical setup as the channel regression. `cargo run --release --example check_readers --
30 1835103083 failure.bin` runs a deterministic mutation smoke test on small synthetic reader
inputs. It is neither coverage-guided fuzzing nor a sanitizer and does not execute shaders.


Static fragments may declare g_Screen (vec3: target width, height and aspect), g_TexelSize and
g_TexelSizeHalf (vec2). They follow the framebuffer dimensions passed to each draw, including
resizes; they do not start animation. Time-driven materials are described below; audio, cursor and camera inputs remain unsupported.
`--shot-size WIDTHxHEIGHT` selects a viewer screenshot size (1–16384 per axis, at most 16
megapixels); the default is 1280×720. Readback flips rows in place and validates the framebuffer
and GL errors before saving. `python3 tools/check-frame-uniforms.py` checks these uniforms and
pixel-sized stripes at landscape, portrait, odd and 1×1 targets.


## Time-driven fragments

`g_Time` and `g_Frametime` are float uniforms supplied by the host. `draw_timed` takes target,
fit, scene time and frame delta; the ordinary draw methods use zero time/delta. The host owns
scheduling, speed and pause: keep time fixed and delta zero while paused. `Renderer::animated()`
is based on active GPU uniform locations, so declarations optimized away by the driver do not
request animation. `prepare_still` refuses time declarations rather than freezing them silently.

Apolline-wall keeps a native renderer only for active time uniforms and uses its existing
shader clock, FPS/quality limits, frame callbacks, pauses and crossfades. Other scenes are
flattened once. This adds fragment animation with static source textures; sprite-sheet
animation remains unsupported.

```sh
python3 tools/demo-scene.py wave --still --wave-demo
cargo run --example view -- wave
cargo run --example view -- --shot wave.png --time 1.0 wave
```

The demo exposes wave_enabled, wave_strength and wave_speed. Disabling the wave yields the baseline pixels and an inactive GPU clock. Zero strength also
matches the baseline; the tested driver retains its time uniform in that variant. The viewer requests
animation frames only while a time uniform or moving parallax needs them. Its --time option
selects deterministic screenshot time; snapshot delta is zero.
`python3 tools/check-time-uniforms.py` checks changing/fixed time and optimized-away uniforms.
The native timed host has been compiled/tested on Arch Linux; real Linux GLES/compositor
playback still requires a runtime pass.


## Project resource roots

On Unix, project and assets directories are opened once and retained as directory descriptors.
Resource lookup walks components with openat and O_NOFOLLOW/O_DIRECTORY before a bounded read;
parent symlink swaps cannot redirect a validated pathname between checking and opening. A
user-selected root may itself be a symlink. Symlinks below that root are refused, including
internal aliases; reopen Project to switch to an atomically replaced directory. A configured
assets root must exist and be readable. Non-Unix builds retain canonical-path checking.
The file reader caps Vec capacity while reading a growing file, and probes only one extra
byte to detect overflow. These are read/allocation limits, not a process memory sandbox.


The viewer's --shot uses Project::prepare_snapshot: texture animation and unsupported
3D layer transforms are refused before rendering; scripts play up to `--time` at 30 frames a
second. Unlike native desktop preparation, snapshots
allow pointer parallax at a neutral eye. This keeps default synthetic parallax fixtures usable
without silently flattening unsupported animation into a reference frame.


## Optional textures and sparse slots

Material textures may contain null or an empty string for an unbound slot; the first texture
is still required. Bound secondary inputs carry explicit TextureInput.slot indices, so slots
0 and 7 can be used without allocating placeholders for 1–6. Shared decoded/uploaded textures
still count once; absent inputs consume no texture bytes. Genericimage retains its single-slot
contract.

For sampler metadata with combo, Project preparation derives the toggle from actual bindings
before selecting conditional code. Plain optional textures omit the toggle when absent;
opacitymask/rgbmask/flowmask modes define it as zero. A bound texture defines it as one.
Contradictory material combo values and duplicate controllers refuse preparation. An unbound
optional sampler/resolution declaration is removed only when selected code never references
it; an active read is an error. Missing named files remain errors, rather than becoming an
absent mask. Default resources are resolved by Project before deriving texture toggles.
Metadata is collected before conditional selection; ambiguous declarations in alternate branches
with conflicting metadata remain outside this subset. Identical metadata in repeated guarded
headers is accepted; active duplicate declarations still fail. Stock assets and editor painting
are not supplied here.

Fragment::prepare_bound accepts a mask of bound slots; its specialization cannot be attached
to different bindings later. Fragment::prepare remains the explicit path without automatic
texture toggles. The image material loader uses prepare_bound. Token pasting is refused so macros cannot hide uniforms from binding validation.
Multiline declaration replacement preserves line counts in compiler diagnostics.

```sh
python3 tools/demo-scene.py masked --still --mask-demo --without-mask
cargo run --example view -- --stats --shot without-mask.png masked
python3 tools/check-optional-textures.py
```

The GPU regression compares masks in slots 1 and 7, counts uploads/bytes, checks missing-mask
static classification and verifies alpha blending at fixed times. It uses synthetic input;
real Workshop/GLES driver behavior still needs validation. Interface semantics follow the public
[optional texture documentation](https://docs.wallpaperengine.io/en/scene/shader/variables.html#optional-visible-texture).


## Shader default resources

Custom material textures may be omitted, an empty list, or contain empty slots. Project fills
unbound slots from sampler metadata default resource names, then requires a resolved slot 0.
Explicit nonempty material entries take precedence; a missing explicit file never falls back
to the annotated default. Genericimage keeps its explicit one-texture contract. Defaults use
the existing scene.pkg → project folder → assets lookup and the shared decoded/GPU budgets.
No stock noise/white assets are generated or shipped. Render-target defaults need another
renderer.

The shader template is loaded/expanded once, used for metadata, then consumed for bound
specialization. Defaults and texture-driven toggles are resolved before conditional selection;
an inactive branch's default can therefore still require its resource in this subset. Conflicting
defaults are refused. Repeated identical guarded metadata does not cause duplicate reads or
bindings. The low-level Fragment API still expects the caller to provide actual bindings.

Material JSON, blending, resolved names, shader preparation and failures are cached for one
preparation, including generic materials. Shared texture decode/read failures are also cached;
monotonically decreasing texture budgets make budget failures valid for that preparation.
Reopening/preparing again observes repaired files and new property values. Cached error text
is bounded before replication to other models, as well as in visible diagnostics. Limits remain
4096 model/material results, 64 custom shader preparation attempts and 16 MiB prepared source.
Unknown generic pass fields and malformed blending now refuse native preparation before decoding.

`python3 tools/check-default-textures.py` checks actual GL pixels, default/explicit overrides,
package/folder/assets precedence, guarded metadata reuse, unique uploads and missing-file refusal.
These synthetic checks do not establish real Workshop or Linux GLES compatibility.
