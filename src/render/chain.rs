//! A layer with effects is drawn into a texture of its own size first: its own material, then
//! each effect pass over the result of the one before, back and forth between two targets.
//! The scene then places the last result like a plain layer. A chain nothing animates is
//! drawn once.

use std::cell::Cell;

use glow::HasContext;

use super::target::Target;
use super::{Input, Program, Step};
use crate::Error;

pub(super) struct Chain {
    targets: [Target; 2],
    size: (u32, u32),
    steps: Vec<Step>,
    animated: bool,
    /// The screen it was last drawn for, which effects can read.
    drawn: Cell<Option<(u32, u32)>>,
}

/// Draws a whole target, with its top row first like an uploaded texture's.
const WHOLE: super::Affine = [[2.0, 0.0, 0.0], [0.0, -2.0, 0.0]];

impl Chain {
    /// `steps` start with the layer's own; each after it reads the one before in slot 0.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn new(
        gl: &glow::Context,
        size: (u32, u32),
        steps: Vec<Step>,
        animated: bool,
    ) -> Result<Self, Error> {
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
                animated,
                drawn: Cell::new(None),
            })
        }
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

    /// The texture the last step draws into, which holds the layer's finished picture.
    pub fn result(&self) -> glow::Texture {
        self.targets[(self.steps.len() - 1) % 2].texture
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
        screen: (u32, u32),
        time: f32,
        delta: f32,
        inputs: &super::material::Inputs,
    ) {
        if self.drawn.get() == Some(screen) && !self.animated {
            return;
        }
        let (width, height) = self.size;
        let whole = (width as f32, height as f32);
        let resolution = [whole.0, whole.1, whole.0, whole.1];
        unsafe {
            gl.disable(glow::BLEND);
            gl.viewport(0, 0, width as i32, height as i32);
            for (at, step) in self.steps.iter().enumerate() {
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
                super::bind(gl, program, step, &WHOLE, whole, [1.0; 4], before.as_ref());
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
