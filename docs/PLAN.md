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

Stages 1 and 2 are written, with a viewer (`examples/view.rs`) that can also save one frame
off screen, the start of the corpus comparison. Everything is tested on files built in the
tests and a synthetic scene drawn on macOS's OpenGL. Not one real `scene.pkg`, `.tex` or
`scene.json` has been read yet, so these are still guesses until then:
- y going up from the bottom in scene coordinates, and angles in radians;
- `normal` blending meaning opaque;
- how far the parallax moves, which is tuned by eye later;
- the animation frames of `TEXS` textures, not read yet.

That needs a machine with Wallpaper Engine installed through Steam, and is the first thing to
do there.
