# Changelog

## 0.1.0 — 2026-11-07

The first release: Wallpaper Engine scenes drawn natively, as a library any GL context can host.

- **Reads** Workshop projects as they come: `project.json`, `scene.pkg` or a plain folder, `.tex` textures (DXT, RGBA, RG, R, LZ4, embedded PNG and JPEG, sprite sheets), materials, effects, particles, puppet-warped models, text layers and SceneScript, with bounds on every size and path.
- **Draws** with glow on OpenGL ES 3 or desktop OpenGL 3.3: layers with 32 blend modes, Wallpaper Engine's shaders turned into GLSL, multi-pass and composed effects, bloom, particles with turbulence, control points and children, puppet warp, timelines, parallax and camera shake following the pointer, and the author's properties.
- **Scripts** run in a QuickJS sandbox per renderer: no filesystem or network, a 64 MiB heap, a deadline each frame.
- **On real scenes**: all 27 of a real Workshop library's scenes that prepare draw on NVIDIA under Linux and match their previews, and 55 of 55 on macOS. A scene that needs what isn't done yet is refused up front with the reason.
- `Renderer::size` gives the scene's own size, for a host that draws one scene for monitors of different shapes.
