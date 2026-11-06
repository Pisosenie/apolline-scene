//! Effects the scene's scripts show and hide, and the values scripts and timelines give them,
//! as it runs.

use std::collections::HashMap;

use super::Drawn;
use crate::scene::{Knob, Object, Scene};

/// Where what the scripts say about effects lands in the drawn layers.
#[derive(Default)]
pub(super) struct Wiring {
    /// For each of the scene's effects, as `Scene::effects` numbers them: the layers and
    /// steps that draw it.
    switches: Vec<Vec<(usize, usize)>>,
    /// For each effect value scripts or timelines drive, as `Scene::driven` lists them: the
    /// layers, steps and constants it sets.
    knobs: Vec<Vec<(usize, usize, usize)>>,
    /// For each instance override they drive, likewise: the particle layers it scales, and
    /// the value it started from, which the system was made with.
    tunes: Vec<Vec<(usize, &'static str, f32)>>,
}

impl Wiring {
    pub fn new(scene: &Scene, layers: &[Drawn]) -> Self {
        let mut by_object: HashMap<usize, Vec<usize>> = HashMap::new();
        for (at, layer) in layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.chain.is_some())
        {
            by_object.entry(layer.object).or_default().push(at);
        }
        let chained = |object: usize| {
            by_object
                .get(&object)
                .into_iter()
                .flatten()
                .filter_map(|&at| Some((at, layers[at].chain.as_ref()?)))
        };
        let switches = scene
            .effects()
            .into_iter()
            .map(|(object, effect)| {
                chained(object)
                    .flat_map(|(at, chain)| {
                        chain
                            .steps_of(effect.index)
                            .map(move |(step, _)| (at, step))
                    })
                    .collect()
            })
            .collect();
        let knobs = scene
            .driven
            .iter()
            .map(|driven| match &driven.knob {
                Knob::Effect { effect, pass, name } => Some((driven.object, *effect, *pass, name)),
                Knob::Override(_) => None,
            })
            .map(|knob| {
                let mut found = Vec::new();
                let Some((object, effect, pass, name)) = knob else {
                    return found;
                };
                for (at, chain) in chained(object) {
                    for (step, drawn) in chain.steps_of(effect) {
                        if drawn.pass != pass {
                            continue;
                        }
                        let keys = drawn.keys.iter().enumerate();
                        found.extend(
                            keys.filter(|(_, key)| key.as_deref() == Some(name.as_str()))
                                .map(|(constant, _)| (at, step, constant)),
                        );
                    }
                }
                found
            })
            .collect();
        let tunes = scene
            .driven
            .iter()
            .map(|driven| {
                let Knob::Override(key) = driven.knob else {
                    return Vec::new();
                };
                let Some(Object::Particles(particles)) = scene.objects.get(driven.object) else {
                    return Vec::new();
                };
                let start = crate::value::number(particles.overrides.get(key)).unwrap_or(1.0);
                layers
                    .iter()
                    .enumerate()
                    .filter(|(_, layer)| layer.object == driven.object && layer.swarm.is_some())
                    .map(|(at, _)| (at, key, start))
                    .collect()
            })
            .collect();
        Wiring {
            switches,
            knobs,
            tunes,
        }
    }

    /// Shows and hides effects and sets their values as the scripts now have them; a layer
    /// whose effects change draws them again.
    pub fn apply(&self, layers: &mut [Drawn], shown: &[bool], values: &[[f32; 4]]) {
        for (switch, &on) in self.switches.iter().zip(shown) {
            for &(at, step) in switch {
                if let Some(chain) = layers[at].chain.as_mut() {
                    chain.show(step, on);
                }
            }
        }
        for (knob, &numbers) in self.knobs.iter().zip(values) {
            for &(at, step, constant) in knob {
                if let Some(chain) = layers[at].chain.as_mut() {
                    chain.set(step, constant, numbers);
                }
            }
        }
        for (tune, &[value, ..]) in self.tunes.iter().zip(values) {
            for &(at, key, start) in tune {
                if let Some(swarm) = &layers[at].swarm
                    && start.abs() > 1e-6
                {
                    swarm.tune(key, value / start);
                }
            }
        }
        for switch in &self.switches {
            for &(at, _) in switch {
                let layer = &mut layers[at];
                if let Some(chain) = &layer.chain {
                    layer.step.inputs[0].texture = chain.result();
                }
            }
        }
    }
}
