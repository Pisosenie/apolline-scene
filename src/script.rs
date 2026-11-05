//! SceneScript: the JavaScript modules a scene puts on its objects' values, run in QuickJS.
//! Each frame every script's `update` gets its value and returns the new one, and scripts may
//! move any layer through `thisScene`. What they can reach is what Wallpaper Engine documents
//! for a wallpaper: the engine's clock, time of day, audio, vectors and the scene's layers.
//! Cursor and media events never come, as on a desktop nobody is pointing at; a script that
//! asks for anything else fails and is named, so the scene can be refused.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use rquickjs::loader::{BuiltinLoader, BuiltinResolver};
use rquickjs::{Context, Ctx, Function, Module, Object, Runtime, Value};

use crate::Error;
use crate::scene::{Local, Node, Object as SceneObject, Pose, Scene, pose};

const WORLD: &str = include_str!("script/world.js");
const MODULES: [(&str, &str); 3] = [
    ("WEMath", include_str!("script/WEMath.js")),
    ("WEColor", include_str!("script/WEColor.js")),
    ("WEVector", include_str!("script/WEVector.js")),
];
/// Each object's state in the world's array: origin, scale, angles in degrees, colour, alpha,
/// brightness and visibility.
const STRIDE: usize = 15;
const MEMORY: usize = 64 * 1024 * 1024;
const STACK: usize = 1024 * 1024;
/// How long loading the scripts and their first frame may take, and each frame after.
const LOADING_MS: u64 = 2000;
const FRAME_MS: u64 = 50;

pub struct Stage {
    // The context holds the runtime; it goes first so it is dropped first.
    context: Context,
    runtime: Runtime,
    deadline: Arc<AtomicU64>,
    started: Instant,
    names: Vec<String>,
    tree: Vec<Node>,
    locals: Vec<Local>,
    poses: Vec<Pose>,
    daytime: bool,
    audio: bool,
    /// The world itself was cut short, not a script in it: nothing runs any more.
    stopped: bool,
}

/// What a frame of the scene's scripts sees.
#[derive(Clone, Copy, Debug)]
pub struct Moment<'a> {
    /// Seconds since the wallpaper started, and since the last frame.
    pub time: f64,
    pub delta: f32,
    /// How far through the local day it is, 0 to 1.
    pub daytime: f32,
    /// The screen's size in pixels.
    pub screen: (u32, u32),
    /// The audio spectrum, left then right, when the host listens.
    pub spectrum: Option<&'a [[f32; 64]; 2]>,
}

impl Stage {
    /// Loads the scene's scripts and runs their first frame at time zero. `user` holds the
    /// project's property values, which scripts read as `engine.userProperties`. A script that
    /// fails to load or throws is an error naming it.
    pub fn new(
        scene: &Scene,
        user: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<Self>, Error> {
        if scene.scripts.is_empty() {
            return Ok(None);
        }
        let fail = |what: rquickjs::Error| Error::Format(format!("SceneScript: {what}"));
        let runtime = Runtime::new().map_err(fail)?;
        runtime.set_memory_limit(MEMORY);
        runtime.set_max_stack_size(STACK);
        let started = Instant::now();
        let deadline = Arc::new(AtomicU64::new(0));
        let watched = Arc::clone(&deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            started.elapsed().as_millis() as u64 > watched.load(Ordering::Relaxed)
        })));
        let mut resolver = BuiltinResolver::default();
        let mut loader = BuiltinLoader::default();
        for (name, source) in MODULES {
            resolver.add_module(name);
            loader.add_module(name, source);
        }
        runtime.set_loader(resolver, loader);
        let context = Context::full(&runtime).map_err(fail)?;
        let names = scene.names.clone();
        let mut stage = Stage {
            context,
            runtime,
            deadline,
            started,
            names,
            tree: scene.tree.clone(),
            locals: scene.locals.clone(),
            poses: pose(&scene.locals, &scene.tree),
            daytime: scene
                .scripts
                .iter()
                .any(|script| script.source.contains("timeOfDay")),
            audio: false,
            stopped: false,
        };
        stage.allow(LOADING_MS);
        let failures = stage.load(scene, user)?;
        if !failures.is_empty() {
            return Err(Error::Format(failures.join("; ")));
        }
        let failures = stage.step(
            Moment {
                time: 0.0,
                delta: 0.0,
                daytime: local_daytime(),
                screen: (scene.size.0 as u32, scene.size.1 as u32),
                spectrum: None,
            },
            true,
        )?;
        if stage.stopped {
            return Err(Error::Format(
                "SceneScript: the scripts ran too long".into(),
            ));
        }
        if !failures.is_empty() {
            return Err(Error::Format(failures.join("; ")));
        }
        Ok(Some(stage))
    }

    /// Whether the scripts read the time of day, and the audio spectrum; a script may start
    /// listening any frame.
    pub fn needs(&self) -> (bool, bool) {
        (self.daytime, self.audio)
    }

    /// Runs a frame of every script and returns where each of the scene's objects is now.
    /// Scripts that throw stop running; the rest go on.
    pub fn tick(&mut self, moment: Moment) -> &[Pose] {
        if !self.stopped {
            // Failures were reported when the scene was loaded; later ones only stop that script.
            let _ = self.step(moment, false);
        }
        &self.poses
    }

    fn allow(&self, milliseconds: u64) {
        let now = self.started.elapsed().as_millis() as u64;
        self.deadline.store(now + milliseconds, Ordering::Relaxed);
    }

    /// Sets up the world and loads each script as a module, returning the ones that failed.
    fn load(
        &mut self,
        scene: &Scene,
        user: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Vec<String>, Error> {
        let objects: Vec<serde_json::Value> = scene
            .objects
            .iter()
            .zip(&self.names)
            .zip(&scene.tree)
            .map(|((object, name), node)| {
                let size = match object {
                    SceneObject::Image(image) => image.size.unwrap_or((0.0, 0.0)),
                    _ => (0.0, 0.0),
                };
                serde_json::json!([name, node.parent.map_or(-1, |at| at as i64), size.0, size.1])
            })
            .collect();
        let initial: Vec<f64> = self.locals.iter().flat_map(state).collect();
        let setup = serde_json::json!({
            "objects": objects,
            "initial": initial,
            "width": scene.size.0,
            "height": scene.size.1,
            "user": user,
        })
        .to_string();
        let names = &self.names;
        self.context.with(|ctx| {
            let fail = |error| Error::Format(format!("SceneScript: {}", caught(&ctx, error)));
            ctx.eval::<(), _>(WORLD).map_err(fail)?;
            let world = world(&ctx).map_err(fail)?;
            let setup_value: Value = ctx.json_parse(setup).map_err(fail)?;
            world
                .get::<_, Function>("setup")
                .and_then(|setup| setup.call::<_, ()>((setup_value,)))
                .map_err(fail)?;
            let add: Function = world.get("add").map_err(fail)?;
            let asking: Function = world.get("asking").map_err(fail)?;
            let mut failures = Vec::new();
            for (at, script) in scene.scripts.iter().enumerate() {
                asking.call::<_, String>(()).map_err(fail)?;
                let properties = serde_json::Value::Object(script.properties.clone());
                // The script's own names come first, on its first line, so its lines keep
                // their numbers.
                let source = format!(
                    "const thisLayer = __world.layer({object}), thisObject = thisLayer, \
                     createScriptProperties = () => __world.properties({properties});{source}",
                    object = script.object,
                    source = script.source,
                );
                let loaded = Module::declare(ctx.clone(), format!("script{at}"), source)
                    .and_then(|module| {
                        let (module, promise) = module.eval()?;
                        promise.finish::<()>()?;
                        module.namespace()
                    })
                    .and_then(|exports| {
                        add.call::<_, ()>((script.object, script.property, exports))
                    });
                if let Err(error) = loaded {
                    let mut message = caught(&ctx, error);
                    let asked: String = asking.call(()).unwrap_or_default();
                    if !asked.is_empty() {
                        message = format!("{message} (it asked for {asked})");
                    }
                    failures.push(failure(
                        names,
                        script.object as i64,
                        script.property,
                        &message,
                    ));
                }
            }
            Ok(failures)
        })
    }

    /// One frame of the scripts, read back into poses, and with `report` the scripts that
    /// have failed so far. Every call into the world runs against a deadline.
    fn step(&mut self, moment: Moment, report: bool) -> Result<Vec<String>, Error> {
        let spectrum: Option<Vec<f32>> = moment.spectrum.map(|bins| bins.concat());
        self.allow(FRAME_MS);
        let names = &self.names;
        let (values, failures, stopped) = self.context.with(|ctx| {
            let fail = |error| Error::Format(format!("SceneScript: {}", caught(&ctx, error)));
            let world = world(&ctx).map_err(fail)?;
            let tick: Function = world.get("tick").map_err(fail)?;
            let mut stopped = false;
            let values: Option<Vec<f64>> = match tick.call((
                moment.time,
                moment.delta,
                moment.daytime,
                moment.screen.0,
                moment.screen.1,
                spectrum,
            )) {
                Ok(values) => Some(values),
                // Cut short: the frame isn't read back, and what ran too long stops. When
                // nothing was running, the world itself is what's slow, and it all stops.
                Err(_) => {
                    let _ = ctx.catch();
                    self.allow(FRAME_MS);
                    let blamed: bool = world
                        .get::<_, Function>("abandon")
                        .and_then(|abandon| abandon.call(()))
                        .unwrap_or(false);
                    stopped = !blamed;
                    None
                }
            };
            let mut failures = Vec::new();
            if report {
                self.allow(FRAME_MS);
                let failed: Vec<Vec<String>> = world
                    .get::<_, Function>("failures")
                    .and_then(|failures| failures.call(()))
                    .map_err(fail)?;
                failures = failed
                    .iter()
                    .filter_map(|failed| match failed.as_slice() {
                        [object, property, message] => Some(failure(
                            names,
                            object.parse().unwrap_or(-1),
                            property,
                            message,
                        )),
                        _ => None,
                    })
                    .collect();
            }
            Ok::<_, Error>((values, failures, stopped))
        })?;
        self.stopped |= stopped;
        if let Some(values) = values {
            let (objects, listening) = values.split_at(values.len().saturating_sub(1));
            self.audio |= listening.first().is_some_and(|&on| on != 0.0);
            for (local, values) in self.locals.iter_mut().zip(objects.as_chunks::<STRIDE>().0) {
                *local = read(values);
            }
            self.poses = pose(&self.locals, &self.tree);
        }
        // Promises the scripts made settle between frames, against the same kind of deadline.
        self.allow(FRAME_MS);
        while self.runtime.is_job_pending() {
            if self.runtime.execute_pending_job().is_err() {
                break;
            }
        }
        Ok(failures)
    }
}

/// How far through the local day it is now, 0 to 1.
fn local_daytime() -> f32 {
    let mut now: libc::tm = unsafe { std::mem::zeroed() };
    let time = unsafe { libc::time(std::ptr::null_mut()) };
    if unsafe { libc::localtime_r(&time, &mut now) }.is_null() {
        return 0.5;
    }
    (now.tm_hour * 3600 + now.tm_min * 60 + now.tm_sec) as f32 / 86400.0
}

fn world<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    ctx.globals().get("__world")
}

fn state(local: &Local) -> [f64; STRIDE] {
    let degrees = local.angles.map(f32::to_degrees);
    let mut out = [0.0; STRIDE];
    for (slot, value) in out.iter_mut().zip(
        local
            .origin
            .iter()
            .chain(&local.scale)
            .chain(&degrees)
            .chain(&local.color),
    ) {
        *slot = f64::from(*value);
    }
    out[12] = f64::from(local.alpha);
    out[13] = f64::from(local.brightness);
    out[14] = if local.visible { 1.0 } else { 0.0 };
    out
}

fn read(values: &[f64; STRIDE]) -> Local {
    let three = |at: usize| {
        [
            values[at] as f32,
            values[at + 1] as f32,
            values[at + 2] as f32,
        ]
    };
    Local {
        origin: three(0),
        scale: three(3),
        angles: three(6).map(f32::to_radians),
        color: three(9),
        alpha: values[12] as f32,
        brightness: values[13] as f32,
        visible: values[14] != 0.0,
    }
}

fn caught(ctx: &Ctx, error: rquickjs::Error) -> String {
    let text = rquickjs::CaughtError::from_error(ctx, error).to_string();
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

fn failure(names: &[String], object: i64, property: &str, message: &str) -> String {
    let message: String = message.chars().take(300).collect();
    match usize::try_from(object).ok().and_then(|at| names.get(at)) {
        Some(name) => {
            let name: String = name.chars().take(80).collect();
            format!("the script on {name}'s {property}: {message}")
        }
        None => format!("a script's {property}: {message}"),
    }
}

#[cfg(test)]
mod tests;
