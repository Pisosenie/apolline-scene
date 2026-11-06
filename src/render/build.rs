//! Turning prepared layers into what the renderer draws: textures uploaded once however many
//! layers use them, one program per distinct shader, and a chain for each layer with effects.

use std::collections::HashMap;
use std::sync::Arc;

use glow::HasContext;

use super::chain::{Chain, Pass};
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
        posed: layer.image.pose(),
        words: None,
        sprite: None,
        compose: layer.compose,
        mode: layer.image.mode,
    };
    drawn.pose(&layer.image.pose());
    drawn
}

/// Effect targets are at most this many pixels on a side.
const TARGET_SIDE: u32 = 8192;

pub(super) struct Builder<'a> {
    gl: &'a glow::Context,
    textures: HashMap<*const Texture, (glow::Texture, [f32; 2])>,
    /// A sprite sheet's images after its first, uploaded once however many layers play it.
    sheets: HashMap<*const Texture, Vec<glow::Texture>>,
    variants: HashMap<(Option<&'a str>, &'a str), usize>,
    /// Our particle program, once a layer needs it.
    particles: Option<usize>,
    /// Our blending layers' programs, by mode.
    blends: HashMap<u8, usize>,
    /// The objects whose finished pictures other layers' effects read.
    pub composited: std::collections::BTreeSet<usize>,
    source_bytes: usize,
    pub allocated: usize,
}

impl<'a> Builder<'a> {
    pub fn new(gl: &'a glow::Context) -> Self {
        Builder {
            gl,
            textures: HashMap::new(),
            sheets: HashMap::new(),
            variants: HashMap::new(),
            particles: None,
            blends: HashMap::new(),
            composited: std::collections::BTreeSet::new(),
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
        let words = layer.text.as_ref().map(|lettering| super::words::Words {
            lettering: Arc::clone(lettering),
            shown: None,
            texture: inputs[0].texture,
            bytes: super::words::bytes(&layer.texture, !layer.effects.is_empty()),
        });
        let sprite = if layer.sprite {
            let key = Arc::as_ptr(&layer.texture);
            if !self.sheets.contains_key(&key) {
                let mut rest = Vec::new();
                for image in 1..layer.texture.images.len() {
                    rest.push(unsafe { self.image(renderer, &layer.texture, image)? });
                }
                self.sheets.insert(key, rest);
            }
            let images: Vec<glow::Texture> = std::iter::once(inputs[0].texture)
                .chain(self.sheets[&key].iter().copied())
                .collect();
            Some(super::sprite::Sprite::new(&layer.texture.frames, &images))
        } else {
            None
        };
        // A sprite sheet's picture is one of its frames; a composition layer over the whole
        // scene is the scene's size.
        let picture = if layer.compose && layer.fullscreen {
            (scene.size.0.round() as u32, scene.size.1.round() as u32)
        } else {
            layer
                .texture
                .frame_size()
                .filter(|_| layer.sprite)
                .unwrap_or((layer.texture.image_width, layer.texture.image_height))
        };
        let own = Step {
            program,
            uv_max: self.textures[&Arc::as_ptr(&layer.texture)].1,
            sheet: None,
            channels: first.channels.count() as i32,
            inputs,
            constants,
        };
        // A layer whose picture others' effects read, or that composes what's under it, is
        // drawn into a target of its own too.
        // A layer that blends is too, for its picture to be mixed with what's under it.
        let blends = layer.image.mode != 0 && layer.particles.is_none();
        let (mut step, chain) = if layer.effects.is_empty()
            && !self.composited.contains(&layer.object)
            && !layer.compose
            && !blends
        {
            (own, None)
        } else {
            let size = (
                picture.0.clamp(1, TARGET_SIDE),
                picture.1.clamp(1, TARGET_SIDE),
            );
            unsafe { self.chain(renderer, scene, layer, own, size)? }
        };
        if blends {
            step.program = unsafe { self.blend(renderer, layer.image.mode)? };
        }
        if let Some(system) = &layer.particles {
            // A script that changes the count may ask for any number up to the most.
            let counted = scene.scripts.iter().any(|script| {
                script.object == layer.object
                    && script.target == crate::scene::Target::Override("count")
            });
            let capacity = if counted {
                crate::particles::MOST
            } else {
                system.max
            };
            return unsafe { self.particles(renderer, layer, system, step, capacity) };
        }
        let image = &layer.image;
        let base = if layer.fullscreen {
            Base::Whole {
                scene: scene.size,
                shade: layer.shade,
            }
        } else {
            let natural = (picture.0 as f32, picture.1 as f32);
            Base::Picture {
                size: image.size.unwrap_or(natural),
                anchor: image.anchor,
                shade: layer.shade,
            }
        };
        let mut drawn = Drawn {
            words,
            sprite,
            ..placed(step, chain, None, layer, base)
        };
        drawn.play(0.0);
        Ok(drawn)
    }

    /// A particle system's layer: its sprite texture through our particle program.
    unsafe fn particles(
        &mut self,
        renderer: &mut Renderer,
        layer: &Layer,
        system: &Arc<crate::particles::System>,
        mut step: Step,
        capacity: usize,
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
        reserve(&mut self.allocated, super::swarm::Swarm::bytes(capacity))?;
        let seed = renderer.layers.len() as u64;
        let frames = &layer.texture.frames;
        let (width, height) = layer
            .texture
            .frame_size()
            .unwrap_or((layer.texture.image_width, layer.texture.image_height));
        let ratio = height as f32 / width.max(1) as f32;
        let swarm = unsafe {
            super::swarm::Swarm::new(self.gl, system, frames, step.uv_max, ratio, seed, capacity)?
        };
        if let Some(behind) = layer.leader {
            let leader = renderer
                .layers
                .len()
                .checked_sub(behind)
                .and_then(|at| renderer.layers.get(at))
                .filter(|leader| leader.object == layer.object)
                .and_then(|leader| leader.swarm.as_ref());
            match leader {
                Some(leader) => swarm.follow(leader),
                None => {
                    unsafe { swarm.delete(self.gl) };
                    return Err(Error::Format(
                        "a particle system following one that isn't drawn".into(),
                    ));
                }
            }
        }
        Ok(placed(
            step,
            None,
            Some(swarm),
            layer,
            Base::Particles {
                shade: layer.shade,
                child: layer.child.as_ref().map(|child| super::Offset {
                    origin: [child.origin[0], child.origin[1]],
                    scale: [child.scale[0], child.scale[1]],
                    angle: child.angles[2],
                }),
            },
        ))
    }

    /// The layer's own step and its effect passes drawn into targets, and the step that places
    /// the result in the scene.
    unsafe fn chain(
        &mut self,
        renderer: &mut Renderer,
        scene: &Scene,
        layer: &'a Layer,
        own: Step,
        size: (u32, u32),
    ) -> Result<(Step, Option<Chain>), Error> {
        let mut steps = vec![own];
        let mut passes = Vec::with_capacity(layer.effects.len());
        // Each effect's buffers, by the effect and their place among its own.
        let mut buffers: Vec<(usize, usize, u32)> = Vec::new();
        let mut buffer = |effect: usize, wanted: crate::project::Buffer| {
            let found = buffers
                .iter()
                .position(|&(owner, at, _)| owner == effect && at == wanted.at);
            found.unwrap_or_else(|| {
                buffers.push((effect, wanted.at, wanted.scale));
                buffers.len() - 1
            })
        };
        for pass in &layer.effects {
            let provided = pass
                .reads
                .iter()
                .map(|(slot, _)| *slot)
                .chain(pass.composites.iter().map(|(slot, _)| *slot))
                .fold(0u8, |mask, slot| mask | 1 << slot);
            let mut bindings = provided | 1;
            let mut inputs = Vec::with_capacity(pass.inputs.len());
            for input in &pass.inputs {
                if input.slot >= 8 || bindings & (1 << input.slot) != 0 {
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
                sheet: None,
                constants,
            });
            let reads = pass
                .reads
                .iter()
                .map(|&(slot, source)| match source {
                    crate::project::Source::Previous => (slot, None),
                    crate::project::Source::Buffer(wanted) => {
                        (slot, Some(buffer(pass.effect, wanted)))
                    }
                })
                .collect();
            // A picture of a layer that isn't there leaves its slot unbound.
            let composites = pass
                .composites
                .iter()
                .filter_map(|&(slot, id)| Some((slot, *scene.ids.get(&id)?)))
                .collect();
            passes.push(Pass {
                effect: pass.effect,
                pass: pass.pass,
                visible: pass.visible,
                keys: pass.fragment.keys.clone(),
                target: pass.target.map(|wanted| buffer(pass.effect, wanted)),
                reads,
                composites,
                unplaced: pass.fragment.unplaced,
            });
        }
        // Effects that read where the layer is on screen draw again as it moves, and those
        // that read other layers' pictures as those change.
        let reads_others = layer.compose || passes.iter().any(|pass| !pass.composites.is_empty());
        let animated = reads_others
            || steps.iter().any(|step| {
                let program = &renderer.programs[step.program];
                program
                    .frame
                    .as_ref()
                    .is_some_and(material::Frame::animated)
                    || program.unproject.is_some()
                    || program.unproject_picture.is_some()
            });
        let scales: Vec<u32> = buffers.iter().map(|&(_, _, scale)| scale).collect();
        let pixels = |(width, height): (u32, u32)| 4 * width as usize * height as usize;
        let bytes = 2 * pixels(size)
            + scales
                .iter()
                .map(|&scale| pixels(super::chain::shrunk(size, scale)))
                .sum::<usize>();
        reserve(&mut self.allocated, bytes)?;
        let chain = unsafe { Chain::new(self.gl, size, steps, passes, &scales, animated)? };
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
            sheet: None,
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

    /// One of an animated texture's images after its first, on the GPU for a sprite sheet's
    /// frames in it.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    unsafe fn image(
        &mut self,
        renderer: &mut Renderer,
        texture: &Texture,
        image: usize,
    ) -> Result<glow::Texture, Error> {
        let bytes: usize = texture.images[image]
            .iter()
            .map(|mip| mip.pixels.len())
            .sum();
        reserve(&mut self.allocated, bytes)?;
        let (uploaded, _) = unsafe { upload::upload_image(self.gl, texture, image)? };
        renderer.textures.push(uploaded);
        Ok(uploaded)
    }

    /// Our program for layers blending in `mode`, made once however many do.
    ///
    /// # Safety
    /// The GL context must be current on this thread.
    unsafe fn blend(&mut self, renderer: &mut Renderer, mode: u8) -> Result<usize, Error> {
        if let Some(&index) = self.blends.get(&mode) {
            return Ok(index);
        }
        let fragment = super::blend::fragment(mode);
        let program = unsafe { super::program(self.gl, super::VERTEX, &fragment)? };
        renderer.programs.push(program);
        let index = renderer.programs.len() - 1;
        self.blends.insert(mode, index);
        Ok(index)
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
