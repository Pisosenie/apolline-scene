//! Turning prepared layers into what the renderer draws: textures uploaded once however many
//! layers use them, one program per distinct shader, and a chain for each layer with effects.

use std::collections::HashMap;
use std::sync::Arc;

use glow::HasContext;

use super::chain::Chain;
use super::{Base, Drawn, Input, Program, Renderer, Step, Values, material, reserve, upload};
use crate::Error;
use crate::project::Layer;
use crate::scene::Scene;
use crate::shader::Fragment;
use crate::tex::Texture;

/// A layer where its object is now.
fn placed(
    step: Step,
    chain: Option<Chain>,
    swarm: Option<super::swarm::Swarm>,
    layer: &Layer,
    base: Base,
) -> Drawn {
    let mut drawn = Drawn {
        step,
        chain,
        swarm,
        origin: [0.0; 2],
        size: (0.0, 0.0),
        angle: 0.0,
        tint: [1.0; 4],
        blending: layer.blending,
        depth: layer.image.depth,
        object: layer.object,
        base,
        visible: true,
    };
    drawn.pose(&layer.image.pose());
    drawn
}

/// Effect targets are at most this many pixels on a side.
const TARGET_SIDE: u32 = 8192;

pub(super) struct Builder<'a> {
    gl: &'a glow::Context,
    textures: HashMap<*const Texture, (glow::Texture, [f32; 2])>,
    variants: HashMap<(Option<&'a str>, &'a str), usize>,
    /// Our particle program, once a layer needs it.
    particles: Option<usize>,
    source_bytes: usize,
    pub allocated: usize,
}

impl<'a> Builder<'a> {
    pub fn new(gl: &'a glow::Context) -> Self {
        Builder {
            gl,
            textures: HashMap::new(),
            variants: HashMap::new(),
            particles: None,
            source_bytes: 0,
            allocated: 0,
        }
    }

    /// # Safety
    /// The GL context must be current on this thread.
    pub unsafe fn layer(
        &mut self,
        renderer: &mut Renderer,
        scene: &Scene,
        layer: &'a Layer,
    ) -> Result<Drawn, Error> {
        let count = 1 + layer.secondary.len();
        if count > 8 || (layer.fragment.is_none() && count != 1) {
            return Err(Error::Format("unsupported number of layer textures".into()));
        }
        let mut bindings = 1u8;
        for input in &layer.secondary {
            if input.slot == 0 || input.slot >= 8 || bindings & (1 << input.slot) != 0 {
                return Err(Error::Format(
                    "duplicate or invalid secondary sampler slot".into(),
                ));
            }
            bindings |= 1 << input.slot;
        }
        if let Some(fragment) = &layer.fragment {
            fragment.validate_inputs(bindings)?;
        }
        let mut inputs = Vec::with_capacity(count);
        for (slot, texture) in std::iter::once((0, &layer.texture)).chain(
            layer
                .secondary
                .iter()
                .map(|input| (input.slot, &input.texture)),
        ) {
            inputs.push(unsafe { self.input(renderer, slot, texture)? });
        }
        let first = layer
            .texture
            .images
            .first()
            .and_then(|image| image.first())
            .ok_or_else(|| Error::Format("layer has no texture image".into()))?;
        let (program, constants) = match &layer.fragment {
            Some(fragment) => unsafe { self.program(renderer, fragment)? },
            None => (0, Vec::new()),
        };
        let own = Step {
            program,
            uv_max: self.textures[&Arc::as_ptr(&layer.texture)].1,
            channels: first.channels.count() as i32,
            inputs,
            constants,
        };
        let (step, chain) = if layer.effects.is_empty() {
            (own, None)
        } else {
            let size = (
                layer.texture.image_width.clamp(1, TARGET_SIDE),
                layer.texture.image_height.clamp(1, TARGET_SIDE),
            );
            unsafe { self.chain(renderer, layer, own, size)? }
        };
        if let Some(system) = &layer.particles {
            return unsafe { self.particles(renderer, layer, system, step) };
        }
        let image = &layer.image;
        let base = if layer.fullscreen {
            Base::Whole {
                scene: scene.size,
                shade: layer.shade,
            }
        } else {
            let natural = (
                layer.texture.image_width as f32,
                layer.texture.image_height as f32,
            );
            Base::Picture {
                size: image.size.unwrap_or(natural),
                anchor: image.anchor,
                shade: layer.shade,
            }
        };
        Ok(placed(step, chain, None, layer, base))
    }

    /// A particle system's layer: its sprite texture through our particle program.
    unsafe fn particles(
        &mut self,
        renderer: &mut Renderer,
        layer: &Layer,
        system: &Arc<crate::particles::System>,
        mut step: Step,
    ) -> Result<Drawn, Error> {
        if step.program != 0 || !layer.effects.is_empty() {
            return Err(Error::Format(
                "particles with a material of their own".into(),
            ));
        }
        step.program = match self.particles {
            Some(index) => index,
            None => {
                let program = unsafe {
                    super::program(self.gl, super::swarm::VERTEX, super::swarm::FRAGMENT)?
                };
                renderer.programs.push(program);
                let index = renderer.programs.len() - 1;
                self.particles = Some(index);
                index
            }
        };
        reserve(&mut self.allocated, super::swarm::Swarm::bytes(system))?;
        let seed = renderer.layers.len() as u64;
        let swarm = unsafe { super::swarm::Swarm::new(self.gl, system, seed)? };
        Ok(placed(step, None, Some(swarm), layer, Base::Particles))
    }

    /// The layer's own step and its effect passes drawn into targets, and the step that places
    /// the result in the scene.
    unsafe fn chain(
        &mut self,
        renderer: &mut Renderer,
        layer: &'a Layer,
        own: Step,
        size: (u32, u32),
    ) -> Result<(Step, Option<Chain>), Error> {
        let mut steps = vec![own];
        for pass in &layer.effects {
            let mut bindings = 1u8;
            let mut inputs = Vec::with_capacity(pass.inputs.len());
            for input in &pass.inputs {
                if input.slot == 0 || input.slot >= 8 || bindings & (1 << input.slot) != 0 {
                    return Err(Error::Format("invalid effect texture slot".into()));
                }
                bindings |= 1 << input.slot;
                inputs.push(unsafe { self.input(renderer, input.slot, &input.texture)? });
            }
            pass.fragment.validate_inputs(bindings)?;
            let (program, constants) = unsafe { self.program(renderer, &pass.fragment)? };
            steps.push(Step {
                program,
                inputs,
                channels: 4,
                uv_max: [1.0; 2],
                constants,
            });
        }
        let animated = steps.iter().any(|step| {
            renderer.programs[step.program]
                .frame
                .as_ref()
                .is_some_and(material::Frame::animated)
        });
        reserve(
            &mut self.allocated,
            2 * 4 * size.0 as usize * size.1 as usize,
        )?;
        let chain = unsafe { Chain::new(self.gl, size, steps, animated)? };
        let (width, height) = (size.0 as f32, size.1 as f32);
        let place = Step {
            program: 0,
            inputs: vec![Input {
                slot: 0,
                texture: chain.result(),
                resolution: [width, height, width, height],
            }],
            channels: 4,
            uv_max: [1.0; 2],
            constants: Vec::new(),
        };
        Ok((place, Some(chain)))
    }

    /// # Safety
    /// The GL context must be current on this thread.
    unsafe fn input(
        &mut self,
        renderer: &mut Renderer,
        slot: u8,
        texture: &Arc<Texture>,
    ) -> Result<Input, Error> {
        let key = Arc::as_ptr(texture);
        let (uploaded, _) = match self.textures.get(&key) {
            Some(uploaded) => *uploaded,
            None => {
                let bytes = texture
                    .images
                    .first()
                    .into_iter()
                    .flatten()
                    .try_fold(0usize, |total, mip| total.checked_add(mip.pixels.len()))
                    .ok_or_else(|| Error::Format("texture byte count overflow".into()))?;
                reserve(&mut self.allocated, bytes)?;
                let uploaded = unsafe { upload(self.gl, texture)? };
                renderer.textures.push(uploaded.0);
                self.textures.insert(key, uploaded);
                uploaded
            }
        };
        let first = texture
            .images
            .first()
            .and_then(|image| image.first())
            .ok_or_else(|| Error::Format("layer has no texture image".into()))?;
        Ok(Input {
            slot,
            texture: uploaded,
            resolution: [
                first.width as f32,
                first.height as f32,
                texture.image_width.min(first.width) as f32,
                texture.image_height.min(first.height) as f32,
            ],
        })
    }

    /// The program for a fragment, shared by every fragment with the same sources, and its
    /// values with where they go in it.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    unsafe fn program(
        &mut self,
        renderer: &mut Renderer,
        fragment: &'a Fragment,
    ) -> Result<(usize, Values), Error> {
        let key = (fragment.vertex.as_deref(), fragment.source.as_str());
        let index = match self.variants.get(&key) {
            Some(index) => *index,
            None => {
                let bytes = fragment.source.len() + fragment.vertex.as_ref().map_or(0, String::len);
                if self.variants.len() >= 256
                    || bytes > (16 * 1024 * 1024usize).saturating_sub(self.source_bytes)
                {
                    return Err(Error::Format(
                        "renderer exceeds its shader program budget".into(),
                    ));
                }
                let program: Program = unsafe { material::program(self.gl, fragment)? };
                self.source_bytes += bytes;
                renderer.programs.push(program);
                let index = renderer.programs.len() - 1;
                self.variants.insert(key, index);
                index
            }
        };
        let id = renderer.programs[index].id;
        let constants = fragment
            .constants
            .iter()
            .map(|(name, value)| unsafe { (self.gl.get_uniform_location(id, name), value.clone()) })
            .collect();
        Ok((index, constants))
    }
}
