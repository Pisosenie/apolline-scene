# apolline-scene

A renderer for Wallpaper Engine scenes, in Rust on GLES, written clean-room. It is a library that draws into any GL context it is handed, so it works on its own and in any program: [apolline-wall](https://github.com/Pisosenie/apolline-wall) uses it to draw scenes next to its pictures, videos and shaders, with the same crossfades, pause and limits.

It reads scenes from your own Wallpaper Engine install and Workshop downloads; nothing of Wallpaper Engine's is shipped here.

## Where it is

Stages 1 and 2 of [`docs/PLAN.md`](docs/PLAN.md): a project read from `scene.pkg`, its folder and Wallpaper Engine's `assets`; textures in every format scenes use; and still scenes drawn: image layers placed, scaled, turned, tinted and blended in order over the scene's clear colour, covering the screen the way a wallpaper does, with the camera's parallax following the pointer. Effects, particles and 3D scenes come next; `Scene::missing` says which of them a scene uses, so a host can hand it elsewhere until then.

```rust
let project = apolline_scene::Project::open(folder, assets)?;
let scene = project.scene()?;
let (layers, _skipped) = project.layers(&scene);
let renderer = unsafe { apolline_scene::Renderer::new(&gl, &scene, &layers)? };
renderer.follow(pointer, seconds); // the parallax
unsafe { renderer.draw(&gl, (width, height)) };
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
