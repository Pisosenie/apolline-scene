# apolline-scene

A renderer for Wallpaper Engine scenes, in Rust on GLES, written clean-room. It is a library that draws into any GL context it is handed, so it works on its own and in any program: [apolline-wall](https://github.com/Pisosenie/apolline-wall) uses it to draw scenes next to its pictures, videos and shaders, with the same crossfades, pause and limits.

It reads scenes from your own Wallpaper Engine install and Workshop downloads; nothing of Wallpaper Engine's is shipped here.

## Where it is

Stage 1 of [`docs/PLAN.md`](docs/PLAN.md): reading `scene.pkg` packages and `.tex` textures (DXT1/3/5, RGBA, RG, R, LZ4, embedded PNG and JPEG). Drawing comes next.

```sh
cargo test
```

## License

MIT or Apache-2.0, at your option.
