//! A layer with effects is drawn into a texture of its own size first: its own material, then
//! each effect pass over the result of the one before, back and forth between two targets.
//! A pass may draw into one of its effect's buffers instead, at a size of their own, and read
//! the picture before it and the buffers in whichever slots it binds them to. The scene then
//! places the last result like a plain layer. A chain nothing animates is drawn once, and
//! again when scripts show or hide its effects or change their values.

use std::cell::Cell;

use glow::HasContext;

use super::target::Target;
use super::{Input, Program, Step};
use crate::Error;

pub(super) struct Chain {
    targets: [Target; 2],
    /// The effects' buffers, each with how many times smaller than the picture it is.
    buffers: Vec<(u32, Target)>,
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
    /// The buffer it draws into, by place among the chain's; none for the picture.
    pub target: Option<usize>,
    /// What it reads, by slot: a buffer, or none for the picture before it.
    pub reads: Vec<(u8, Option<usize>)>,
    /// Other layers' finished pictures it reads, by slot and the layer's object.
    pub composites: Vec<(u8, usize)>,
    /// Its vertex shader puts its quad where it goes without the matrix, as passes into
    /// buffers are drawn.
    pub unplaced: bool,
}

/// Layers' finished pictures, by object, which other layers' effects read.
pub(super) type Pictures = [(usize, glow::Texture, (u32, u32))];

/// What a run of the chain is drawn for: the screen, the time, and where the layer is on the
/// screen, which effects that follow the pointer read.
#[derive(Clone, Copy)]
pub(super) struct Moment {
    pub screen: (u32, u32),
    pub time: f32,
    pub delta: f32,
    pub placed: super::Affine,
    /// For a composition layer, the frame drawn so far and where in it the layer's picture
    /// is, which its own step reads in place of its texture.
    pub own: Option<(glow::Texture, [f32; 6])>,
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
        scales: &[u32],
        animated: bool,
    ) -> Result<Self, Error> {
        let shown = std::iter::once(true)
            .chain(passes.iter().map(|pass| pass.visible))
            .collect();
        unsafe {
            let mut made: Vec<Target> = Vec::new();
            let sizes = [size, size]
                .into_iter()
                .chain(scales.iter().map(|&scale| shrunk(size, scale)));
            for wanted in sizes {
                match Target::new(gl, wanted) {
                    Ok(target) => made.push(target),
                    Err(error) => {
                        for target in &made {
                            target.delete(gl);
                        }
                        return Err(error);
                    }
                }
            }
            let mut made = made.into_iter();
            let targets = [made.next(), made.next()].map(|target| target.expect("two made"));
            Ok(Chain {
                targets,
                buffers: scales.iter().copied().zip(made).collect(),
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

    /// The size it draws its picture at.
    pub fn size(&self) -> (u32, u32) {
        self.size
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
            for (scale, buffer) in &self.buffers {
                unsafe { buffer.resize(gl, shrunk(size, *scale)) };
            }
            self.size = size;
        }
        self.drawn.set(None);
    }

    /// The texture the last step drawn into the picture draws into, which holds the layer's
    /// finished picture.
    pub fn result(&self) -> glow::Texture {
        let drawn = self
            .shown
            .iter()
            .enumerate()
            .filter(|&(at, &shown)| shown && (at == 0 || self.passes[at - 1].target.is_none()))
            .count();
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
        pictures: &Pictures,
    ) {
        let Moment {
            screen,
            time,
            delta,
            placed,
            own,
        } = at;
        let composed = own.map(|(frame, place)| {
            let mut step = self.steps[0].clone();
            step.inputs[0].texture = frame;
            step.sheet = Some(place);
            step
        });
        if self.drawn.get() == Some(screen) && !self.animated {
            return;
        }
        let resolution = |(width, height): (u32, u32)| {
            let (width, height) = (width as f32, height as f32);
            [width, height, width, height]
        };
        unsafe {
            gl.disable(glow::BLEND);
            // How many steps have drawn into the picture so far.
            let mut drawn = 0;
            for (at, step) in self.steps.iter().enumerate() {
                if !self.shown[at] {
                    continue;
                }
                let step = match (&composed, at) {
                    (Some(composed), 0) => composed,
                    _ => step,
                };
                let pass = at.checked_sub(1).map(|at| &self.passes[at]);
                let (target, size) = match pass.and_then(|pass| pass.target) {
                    Some(buffer) => (
                        &self.buffers[buffer].1,
                        shrunk(self.size, self.buffers[buffer].0),
                    ),
                    None => (&self.targets[drawn % 2], self.size),
                };
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
                gl.viewport(0, 0, size.0 as i32, size.1 as i32);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                let program = &programs[step.program];
                gl.use_program(Some(program.id));
                gl.bind_vertex_array(Some(program.vao));
                if let Some(frame) = &program.frame {
                    frame.update(gl, screen, size, time, delta, inputs);
                }
                let others = pass
                    .into_iter()
                    .flat_map(|pass| &pass.composites)
                    .filter_map(|&(slot, object)| {
                        let &(_, texture, size) =
                            pictures.iter().find(|(owner, ..)| *owner == object)?;
                        Some(Input {
                            slot,
                            texture,
                            resolution: resolution(size),
                        })
                    });
                let reads: Vec<Input> = pass
                    .into_iter()
                    .flat_map(|pass| &pass.reads)
                    .map(|&(slot, read)| match read {
                        Some(buffer) => {
                            let (scale, buffer) = &self.buffers[buffer];
                            Input {
                                slot,
                                texture: buffer.texture,
                                resolution: resolution(shrunk(self.size, *scale)),
                            }
                        }
                        None => Input {
                            slot,
                            texture: self.targets[(drawn + 1) % 2].texture,
                            resolution: resolution(self.size),
                        },
                    })
                    .chain(others)
                    .collect();
                // A pass into a buffer gets its quad already in place, flipped like the rest, so
                // its matrix comes out the identity; one into the picture, in its pixels.
                let units = match pass {
                    Some(pass) if pass.target.is_some() || pass.unplaced => (2.0, -2.0),
                    _ => (size.0 as f32, size.1 as f32),
                };
                super::bind(gl, program, step, &WHOLE, &placed, units, [1.0; 4], &reads);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                if pass.is_none_or(|pass| pass.target.is_none()) {
                    drawn += 1;
                }
            }
        }
        self.drawn.set(Some(screen));
    }

    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn delete(&self, gl: &glow::Context) {
        unsafe {
            for target in self
                .targets
                .iter()
                .chain(self.buffers.iter().map(|(_, buffer)| buffer))
            {
                target.delete(gl);
            }
        }
    }
}

/// A picture's size `scale` times smaller each way, at least a pixel.
pub(super) fn shrunk(size: (u32, u32), scale: u32) -> (u32, u32) {
    let scale = scale.max(1);
    (size.0.div_ceil(scale).max(1), size.1.div_ceil(scale).max(1))
}
