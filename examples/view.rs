//! Opens a scene in a window: `cargo run --example view -- <project folder> [assets folder]`,
//! where the assets folder is Wallpaper Engine's `assets`. The pointer drives the parallax.
//! With `--shot out.png` first it draws one 1280×720 frame off screen, saves it and quits.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::time::Instant;

use apolline_scene::{Project, Renderer};
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext,
    Version,
};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

struct View {
    project: PathBuf,
    assets: Option<PathBuf>,
    properties: Option<PathBuf>,
    shot: Option<PathBuf>,
    shot_size: (u32, u32),
    shot_time: f32,
    open: Option<Open>,
    pointer: [f32; 2],
    last: Instant,
    failed: bool,
    stats: bool,
    /// A screenshot of whatever layers can be drawn, for looking at scenes not drawn whole.
    partial: bool,
}

struct Open {
    window: Window,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    gl: glow::Context,
    renderer: Renderer,
    started: Instant,
}

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let stats = args
        .iter()
        .position(|arg| arg == "--stats")
        .map(|at| args.remove(at))
        .is_some();
    let partial = args
        .iter()
        .position(|arg| arg == "--partial")
        .map(|at| args.remove(at))
        .is_some();
    let shot_time = match args.iter().position(|arg| arg == "--time") {
        Some(at) => {
            let value = args
                .get(at + 1)
                .and_then(|value| value.to_str())
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && *value >= 0.0);
            let Some(value) = value else {
                eprintln!("view: --time needs a finite nonnegative number of seconds");
                std::process::exit(2);
            };
            args.remove(at + 1);
            args.remove(at);
            value
        }
        None => 0.0,
    };
    let shot_size = match args.iter().position(|arg| arg == "--shot-size") {
        Some(at) => {
            let parsed = args
                .get(at + 1)
                .and_then(|value| value.to_str())
                .and_then(shot_size);
            let Some(size) = parsed else {
                eprintln!(
                    "view: --shot-size needs WIDTHxHEIGHT, dimensions 1–16384, at most 16 megapixels"
                );
                std::process::exit(2);
            };
            args.remove(at + 1);
            args.remove(at);
            size
        }
        None => (1280, 720),
    };
    let properties = match args.iter().position(|arg| arg == "--properties") {
        Some(at) if at + 1 < args.len() => {
            let path = args.remove(at + 1);
            args.remove(at);
            Some(PathBuf::from(path))
        }
        _ => None,
    };
    let shot = match args.iter().position(|arg| arg == "--shot") {
        Some(at) if at + 1 < args.len() => {
            let path = args.remove(at + 1);
            args.remove(at);
            Some(PathBuf::from(path))
        }
        _ => None,
    };
    let mut args = args.into_iter();
    let Some(project) = args.next() else {
        eprintln!(
            "usage: view [--stats] [--shot out.png] [--partial] [--shot-size WIDTHxHEIGHT] [--time SECONDS] [--properties values.json] <project folder> [assets folder]"
        );
        std::process::exit(2);
    };
    let mut view = View {
        properties,
        project: project.into(),
        assets: args.next().map(PathBuf::from),
        shot,
        shot_size,
        shot_time,
        open: None,
        pointer: [0.0; 2],
        last: Instant::now(),
        failed: false,
        stats,
        partial,
    };
    let ran = EventLoop::new()
        .map_err(|err| err.to_string())
        .and_then(|event_loop| event_loop.run_app(&mut view).map_err(|err| err.to_string()));
    if let Err(err) = ran {
        eprintln!("view: {err}");
        std::process::exit(1);
    }
    if view.failed {
        std::process::exit(1);
    }
}

impl View {
    fn open(&self, event_loop: &ActiveEventLoop) -> Result<Open, String> {
        let mut project =
            Project::open(&self.project, self.assets.as_deref()).map_err(|err| err.to_string())?;
        if let Some(path) = &self.properties {
            use std::io::Read;
            let file = std::fs::File::open(path).map_err(|err| err.to_string())?;
            let mut bytes = Vec::new();
            file.take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|err| err.to_string())?;
            if bytes.len() > 1024 * 1024 {
                return Err("property overrides exceed 1 MiB".into());
            }
            let values = serde_json::from_slice(&bytes)
                .map_err(|err| format!("{}: {err}", path.display()))?;
            project
                .set_properties(&values)
                .map_err(|err| err.to_string())?;
        }
        let (scene, layers, skipped) = if self.shot.is_some() && !self.partial {
            let prepared = project
                .prepare_snapshot()
                .map_err(|error| error.to_string())?;
            (prepared.scene, prepared.layers, Vec::new())
        } else {
            let scene = project.scene().map_err(|error| error.to_string())?;
            let (layers, skipped) = project.layers(&scene);
            (scene, layers, skipped)
        };
        for layer in &skipped {
            eprintln!("view: left out {layer}");
        }
        let missing = scene.missing();
        if !missing.is_empty() {
            let named: Vec<String> = missing.iter().map(ToString::to_string).collect();
            eprintln!("view: not drawn yet: {}", named.join(", "));
        }
        if self.shot.is_some() && !self.partial && (!skipped.is_empty() || !missing.is_empty()) {
            return Err("refusing a partial scene screenshot; resolve the reported unsupported resources first".into());
        }
        let title = if project.title.is_empty() {
            "Scene"
        } else {
            &project.title
        };
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(scene.size.0 / 2.0, scene.size.1 / 2.0))
            .with_visible(self.shot.is_none());
        let (window, config) = DisplayBuilder::new()
            .with_window_attributes(Some(attributes))
            .build(event_loop, ConfigTemplateBuilder::new(), |configs| {
                configs
                    .reduce(|best, next| {
                        if next.num_samples() > best.num_samples() {
                            next
                        } else {
                            best
                        }
                    })
                    .expect("glutin offers at least one config")
            })
            .map_err(|err| err.to_string())?;
        let window = window.ok_or("no window")?;
        let display = config.display();
        let handle = window
            .window_handle()
            .map_err(|err| err.to_string())?
            .as_raw();
        // GLES where there is EGL, desktop OpenGL elsewhere, macOS included.
        let gles = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
            .build(Some(handle));
        let desktop = ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .with_profile(GlProfile::Core)
            .build(Some(handle));
        let context = unsafe {
            display
                .create_context(&config, &gles)
                .or_else(|_| display.create_context(&config, &desktop))
        }
        .map_err(|err| format!("a GL context: {err}"))?;
        let surface_attributes = window
            .build_surface_attributes(Default::default())
            .map_err(|err| err.to_string())?;
        let surface = unsafe { display.create_window_surface(&config, &surface_attributes) }
            .map_err(|err| format!("a GL surface: {err}"))?;
        let context = context
            .make_current(&surface)
            .map_err(|err| err.to_string())?;
        let _ = surface.set_swap_interval(&context, SwapInterval::Wait(NonZeroU32::MIN));
        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|name| display.get_proc_address(name))
        };
        let renderer =
            unsafe { Renderer::new(&gl, &scene, &layers) }.map_err(|err| err.to_string())?;
        if self.stats {
            eprintln!(
                "view: {} texture uploads, {} logical texture bytes, animated={}",
                renderer.texture_count(),
                renderer.texture_bytes(),
                renderer.animated()
            );
        }
        Ok(Open {
            window,
            surface,
            context,
            gl,
            renderer,
            started: Instant::now(),
        })
    }
}

impl ApplicationHandler for View {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.open.is_some() {
            return;
        }
        match self.open(event_loop) {
            Ok(open) if self.shot.is_some() => {
                let saved = self
                    .shot
                    .as_deref()
                    .map(|path| shoot(&open, path, self.shot_size, self.shot_time));
                if let Some(Err(err)) = saved {
                    eprintln!("view: {err}");
                    self.failed = true;
                }
                unsafe { open.renderer.delete(&open.gl) };
                event_loop.exit();
            }
            Ok(open) => self.open = Some(open),
            Err(err) => {
                eprintln!("view: {err}");
                self.failed = true;
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(open) = &mut self.open else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(w), Some(h)) =
                    (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                {
                    open.surface.resize(&open.context, w, h);
                }
                open.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let size = open.window.inner_size();
                self.pointer = [
                    position.x as f32 / size.width.max(1) as f32 * 2.0 - 1.0,
                    1.0 - position.y as f32 / size.height.max(1) as f32 * 2.0,
                ];
                open.window.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let seconds = now.duration_since(self.last).as_secs_f32();
                self.last = now;
                let moving = open.renderer.follow(self.pointer, seconds);
                let size = open.window.inner_size();
                unsafe {
                    open.renderer.draw_timed(
                        &open.gl,
                        (size.width, size.height),
                        apolline_scene::render::Fit::Cover,
                        open.started.elapsed().as_secs_f32(),
                        seconds.min(0.25),
                    )
                };
                let _ = open.surface.swap_buffers(&open.context);
                if moving || open.renderer.animated() {
                    open.window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(open) = self.open.take() {
            unsafe { open.renderer.delete(&open.gl) };
        }
    }
}

fn shot_size(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once('x')?;
    let size: (u32, u32) = (width.parse().ok()?, height.parse().ok()?);
    ((1..=16384).contains(&size.0)
        && (1..=16384).contains(&size.1)
        && u64::from(size.0) * u64::from(size.1) <= 16 * 1024 * 1024)
        .then_some(size)
}

/// One frame drawn into an offscreen framebuffer and saved as a PNG, whatever the window.
fn shoot(open: &Open, path: &std::path::Path, (w, h): (u32, u32), time: f32) -> Result<(), String> {
    use glow::HasContext;
    let gl = &open.gl;
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    unsafe {
        let max = gl.get_parameter_i32(glow::MAX_RENDERBUFFER_SIZE).max(0) as u32;
        if w > max || h > max {
            return Err(format!("screenshot dimensions exceed the GL limit {max}"));
        }
        let framebuffer = gl.create_framebuffer()?;
        let colour = match gl.create_renderbuffer() {
            Ok(colour) => colour,
            Err(error) => {
                gl.delete_framebuffer(framebuffer);
                return Err(error);
            }
        };
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(colour));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, w as i32, h as i32);
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::RENDERBUFFER,
            Some(colour),
        );
        let result = (|| {
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!(
                    "incomplete screenshot framebuffer (GL {status:#x})"
                ));
            }
            open.renderer
                .draw_timed(gl, (w, h), apolline_scene::render::Fit::Cover, time, 0.0);
            gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
            gl.pixel_store_i32(glow::PACK_ROW_LENGTH, 0);
            gl.pixel_store_i32(glow::PACK_SKIP_ROWS, 0);
            gl.pixel_store_i32(glow::PACK_SKIP_PIXELS, 0);
            gl.pixel_store_i32(glow::PACK_ALIGNMENT, 1);
            gl.read_pixels(
                0,
                0,
                w as i32,
                h as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );
            let error = gl.get_error();
            if error != glow::NO_ERROR {
                return Err(format!("drawing the screenshot failed (GL {error:#x})"));
            }
            Ok(())
        })();
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        gl.delete_framebuffer(framebuffer);
        gl.delete_renderbuffer(colour);
        result?;
    }
    // Reverse rows in place; a second full frame would double readback memory.
    let stride = w as usize * 4;
    for row in 0..h as usize / 2 {
        let (head, tail) = pixels.split_at_mut((h as usize - row - 1) * stride);
        head[row * stride..(row + 1) * stride].swap_with_slice(&mut tail[..stride]);
    }
    image::RgbaImage::from_raw(w, h, pixels)
        .ok_or("a frame of the wrong size")?
        .save(path)
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::shot_size;

    #[test]
    fn screenshot_dimensions_bound_both_axes_and_total_allocation() {
        for text in [
            "0x1",
            "1x0",
            "16385x1",
            "8192x8192",
            "4294967295x4294967295",
            "1280",
            "-1x720",
        ] {
            assert_eq!(shot_size(text), None, "{text}");
        }
        assert_eq!(shot_size("1280x720"), Some((1280, 720)));
        assert_eq!(shot_size("16384x1024"), Some((16384, 1024)));
    }
}
