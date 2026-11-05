# apolline-scene

A renderer for Wallpaper Engine scenes, in Rust on GLES, written clean-room. It is a library that draws into any GL context it is handed, so it works on its own and in any program: [apolline-wall](https://github.com/Pisosenie/apolline-wall) uses it to draw scenes next to its pictures, videos and shaders, with the same crossfades, pause and limits.

It reads scenes from your own Wallpaper Engine install and Workshop downloads; nothing of Wallpaper Engine's is shipped here.

## Where it is

Stages 1 and 2 of [`docs/PLAN.md`](docs/PLAN.md) and the start of stage 3: a project read from `scene.pkg`, its folder and Wallpaper Engine's `assets`; textures in every format scenes use; still scenes with layers placed, scaled, turned, tinted and blended over the scene's clear colour, covering the screen the way a wallpaper does, with the camera's parallax; user properties; and one-pass fragment materials, time-driven ones included. `Scene::missing` and `Project::prepare` say what a scene uses that isn't drawn yet, so a host can hand it elsewhere. [`docs/NOTES.md`](docs/NOTES.md) has the details a host needs.

```rust
let mut project = apolline_scene::Project::open(folder, assets)?;
project.set_properties(&saved_edits)?;
let prepared = project.prepare()?;
```

It draws with OpenGL ES 3 or desktop OpenGL 3.3, whichever context it is given.

## Trying it

```sh
cargo test
cargo run --example view -- <project folder> [Wallpaper Engine's assets folder]
cargo run --example view -- --shot frame.png <project folder>   # one frame to a PNG, no window
```

## License

MIT or Apache-2.0, at your option.
