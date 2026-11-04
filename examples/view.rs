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
    shot: Option<PathBuf>,
    open: Option<Open>,
    pointer: [f32; 2],
    last: Instant,
}

struct Open {
    window: Window,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    gl: glow::Context,
    renderer: Renderer,
}

fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
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
        eprintln!("usage: view [--shot out.png] <project folder> [assets folder]");
        std::process::exit(2);
    };
    let mut view = View {
        project: project.into(),
        assets: args.next().map(PathBuf::from),
        shot,
        open: None,
        pointer: [0.0; 2],
        last: Instant::now(),
    };
    let ran = EventLoop::new()
        .map_err(|err| err.to_string())
        .and_then(|event_loop| event_loop.run_app(&mut view).map_err(|err| err.to_string()));
    if let Err(err) = ran {
        eprintln!("view: {err}");
        std::process::exit(1);
    }
}

impl View {
    fn open(&self, event_loop: &ActiveEventLoop) -> Result<Open, String> {
        let project =
            Project::open(&self.project, self.assets.as_deref()).map_err(|err| err.to_string())?;
        let scene = project.scene().map_err(|err| err.to_string())?;
        let (layers, skipped) = project.layers(&scene);
        for layer in &skipped {
            eprintln!("view: left out {layer}");
        }
        let missing = scene.missing();
        if !missing.is_empty() {
            eprintln!("view: not drawn yet: {}", missing.join(", "));
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
        Ok(Open {
            window,
            surface,
            context,
            gl,
            renderer,
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
                let saved = self.shot.as_deref().map(|path| shoot(&open, path));
                if let Some(Err(err)) = saved {
                    eprintln!("view: {err}");
                }
                unsafe { open.renderer.delete(&open.gl) };
                event_loop.exit();
            }
            Ok(open) => self.open = Some(open),
            Err(err) => {
                eprintln!("view: {err}");
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
                unsafe { open.renderer.draw(&open.gl, (size.width, size.height)) };
                let _ = open.surface.swap_buffers(&open.context);
                if moving {
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

/// One frame drawn into an offscreen framebuffer and saved as a PNG, whatever the window.
fn shoot(open: &Open, path: &std::path::Path) -> Result<(), String> {
    use glow::HasContext;
    let (w, h) = (1280u32, 720u32);
    let gl = &open.gl;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    unsafe {
        let framebuffer = gl.create_framebuffer()?;
        let colour = gl.create_renderbuffer()?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(colour));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::RGBA8, w as i32, h as i32);
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::RENDERBUFFER,
            Some(colour),
        );
        open.renderer.draw(gl, (w, h));
        gl.read_pixels(
            0,
            0,
            w as i32,
            h as i32,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelPackData::Slice(Some(&mut pixels)),
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.delete_framebuffer(framebuffer);
        gl.delete_renderbuffer(colour);
    }
    // GL reads rows from the bottom up.
    let rows: Vec<u8> = pixels
        .chunks_exact((w * 4) as usize)
        .rev()
        .flatten()
        .copied()
        .collect();
    image::RgbaImage::from_raw(w, h, rows)
        .ok_or("a frame of the wrong size")?
        .save(path)
        .map_err(|err| err.to_string())
}
