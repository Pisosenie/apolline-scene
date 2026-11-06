//! A layer with effects is drawn into a texture of its own size first: its own material, then
//! each effect pass over the result of the one before, back and forth between two targets.
//! The scene then places the last result like a plain layer. A chain nothing animates is
//! drawn once, and again when scripts show or hide its effects or change their values.

use std::cell::Cell;

use glow::HasContext;

use super::target::Target;
use super::{Input, Program, Step};
use crate::Error;

pub(super) struct Chain {
    targets: [Target; 2],
    size: (u32, u32),
    steps: Vec<Step>,
    /// What each step after the layer's own draws, and whether each step is drawn.
    passes: Vec<Pass>,
    shown: Vec<bool>,
    animated: bool,
    /// The screen it was last drawn for, which effects can read.
    drawn: Cell<Option<(u32, u32)>>,
}

/// Which pass of which of the layer's effects a step draws, and the material values its
/// constants come from.
pub(super) struct Pass {
    pub effect: usize,
    pub pass: usize,
    pub visible: bool,
    pub keys: Vec<Option<String>>,
}

/// What a run of the chain is drawn for: the screen, the time, and where the layer is on the
/// screen, which effects that follow the pointer read.
#[derive(Clone, Copy)]
pub(super) struct Moment {
    pub screen: (u32, u32),
    pub time: f32,
    pub delta: f32,
    pub placed: super::Affine,
}

/// Draws a whole target, with its top row first like an uploaded texture's.
const WHOLE: super::Affine = [[2.0, 0.0, 0.0], [0.0, -2.0, 0.0]];

impl Chain {
    /// `steps` start with the layer's own; each after it, one of `passes`, reads the one
    /// drawn before it in slot 0.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn new(
        gl: &glow::Context,
        size: (u32, u32),
        steps: Vec<Step>,
        passes: Vec<Pass>,
        animated: bool,
    ) -> Result<Self, Error> {
        let shown = std::iter::once(true)
            .chain(passes.iter().map(|pass| pass.visible))
            .collect();
        unsafe {
            let first = Target::new(gl, size)?;
            let second = match Target::new(gl, size) {
                Ok(second) => second,
                Err(error) => {
                    first.delete(gl);
                    return Err(error);
                }
            };
            Ok(Chain {
                targets: [first, second],
                size,
                steps,
                passes,
                shown,
                animated,
                drawn: Cell::new(None),
            })
        }
    }

    /// Draws again next time, whatever else has or hasn't changed.
    pub fn touch(&self) {
        self.drawn.set(None);
    }

    /// The layer's own step, which reads its picture.
    pub fn own(&mut self) -> &mut Step {
        &mut self.steps[0]
    }

    /// Draws at a new size from here on, into the same textures.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn resize(&mut self, gl: &glow::Context, size: (u32, u32)) {
        if size != self.size {
            for target in &self.targets {
                unsafe { target.resize(gl, size) };
            }
            self.size = size;
        }
        self.drawn.set(None);
    }

    /// The texture the last step drawn draws into, which holds the layer's finished picture.
    pub fn result(&self) -> glow::Texture {
        let drawn = self.shown.iter().filter(|&&shown| shown).count();
        self.targets[(drawn - 1) % 2].texture
    }

    /// The steps that draw a pass of the layer's effect `effect`, with the pass each draws.
    pub fn steps_of(&self, effect: usize) -> impl Iterator<Item = (usize, &Pass)> {
        self.passes
            .iter()
            .enumerate()
            .filter(move |(_, pass)| pass.effect == effect)
            .map(|(at, pass)| (at + 1, pass))
    }

    /// Shows or hides an effect pass's step; it's drawn again when that changes anything.
    pub fn show(&mut self, step: usize, shown: bool) {
        if step > 0 && self.shown.get(step).is_some_and(|&was| was != shown) {
            self.shown[step] = shown;
            self.drawn.set(None);
        }
    }

    /// Gives one of a step's constants new numbers, as many as it takes.
    pub fn set(&mut self, step: usize, constant: usize, numbers: [f32; 4]) {
        let Some((_, value)) = self
            .steps
            .get_mut(step)
            .and_then(|step| step.constants.get_mut(constant))
        else {
            return;
        };
        let now = value.with(numbers);
        if now != *value {
            *value = now;
            self.drawn.set(None);
        }
    }

    /// Draws the chain unless nothing in it has changed since it was last drawn for this
    /// screen. Leaves its
    /// own framebuffer bound.
    ///
    /// # Safety
    /// The GL context must be current on this thread, with depth, scissor and stencil tests
    /// off and all colour channels writable.
    pub unsafe fn run(
        &self,
        gl: &glow::Context,
        programs: &[Program],
        at: Moment,
        inputs: &super::material::Inputs,
    ) {
        let Moment {
            screen,
            time,
            delta,
            placed,
        } = at;
        if self.drawn.get() == Some(screen) && !self.animated {
            return;
        }
        let (width, height) = self.size;
        let whole = (width as f32, height as f32);
        let resolution = [whole.0, whole.1, whole.0, whole.1];
        unsafe {
            gl.disable(glow::BLEND);
            gl.viewport(0, 0, width as i32, height as i32);
            let shown = self
                .steps
                .iter()
                .zip(&self.shown)
                .filter(|(_, shown)| **shown);
            for (at, (step, _)) in shown.enumerate() {
                let target = &self.targets[at % 2];
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                let program = &programs[step.program];
                gl.use_program(Some(program.id));
                gl.bind_vertex_array(Some(program.vao));
                if let Some(frame) = &program.frame {
                    frame.update(gl, screen, self.size, time, delta, inputs);
                }
                let before = (at > 0).then(|| Input {
                    slot: 0,
                    texture: self.targets[(at - 1) % 2].texture,
                    resolution,
                });
                super::bind(
                    gl,
                    program,
                    step,
                    &WHOLE,
                    &placed,
                    whole,
                    [1.0; 4],
                    before.as_ref(),
                );
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
        }
        self.drawn.set(Some(screen));
    }

    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            for target in &self.targets {
                target.delete(gl);
            }
        }
    }
}
