//! Where a puppet's bones are at a moment, and so where its corners are: each bone's pose is
//! its animations' frames mixed over its rest, placed through its parents, and each corner
//! moves with the bones it hangs from as far as it hangs from each.

use super::{Animation, Mode, Puppet};

/// A bone's place from its parent: moved, turned about the view axis in radians, and scaled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: [f32; 2],
    pub angle: f32,
    pub scale: [f32; 2],
}

/// An animation playing on a puppet, as a layer's `animationlayers` lists it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posed {
    pub animation: u32,
    /// How fast it plays, 1 as made.
    pub rate: f32,
    /// How much of it shows over what's under it, 0 to 1.
    pub blend: f32,
    /// It adds its moves from the rest pose to what's under it rather than replacing it.
    pub additive: bool,
}

/// A flat affine map, applied to a row: `[x y 1]` times `[a b; c d; tx ty]`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Affine([f32; 6]);

impl Pose {
    /// The pose a 4×4 bone matrix holds, turned only about the view axis.
    pub(super) fn from_matrix(matrix: &[f32; 16]) -> Pose {
        let [a, b, c, d] = [matrix[0], matrix[1], matrix[4], matrix[5]];
        let across = a.hypot(b);
        let flipped = a * d - b * c < 0.0;
        Pose {
            position: [matrix[12], matrix[13]],
            angle: b.atan2(a),
            scale: [across, if flipped { -c.hypot(d) } else { c.hypot(d) }],
        }
    }

    fn affine(&self) -> Affine {
        let (sin, cos) = self.angle.sin_cos();
        let [sx, sy] = self.scale;
        Affine([
            sx * cos,
            sx * sin,
            -sy * sin,
            sy * cos,
            self.position[0],
            self.position[1],
        ])
    }

    fn mix(&self, other: &Pose, along: f32) -> Pose {
        let lerp = |a: f32, b: f32| a + (b - a) * along;
        Pose {
            position: [0, 1].map(|axis| lerp(self.position[axis], other.position[axis])),
            angle: lerp(self.angle, other.angle),
            scale: [0, 1].map(|axis| lerp(self.scale[axis], other.scale[axis])),
        }
    }

    /// This with `other`'s change from `rest` added, `along` of it.
    fn add(&self, other: &Pose, rest: &Pose, along: f32) -> Pose {
        let added = |own: f32, to: f32, from: f32| own + (to - from) * along;
        Pose {
            position: [0, 1].map(|axis| {
                added(
                    self.position[axis],
                    other.position[axis],
                    rest.position[axis],
                )
            }),
            angle: added(self.angle, other.angle, rest.angle),
            scale: [0, 1].map(|axis| {
                let from = rest.scale[axis];
                let factor = if from.abs() > 1e-6 {
                    other.scale[axis] / from
                } else {
                    1.0
                };
                self.scale[axis] * (1.0 + (factor - 1.0) * along)
            }),
        }
    }
}

impl Affine {
    /// This, then `then`.
    fn then(&self, then: &Affine) -> Affine {
        let [a, b, c, d, x, y] = self.0;
        let [e, f, g, h, u, v] = then.0;
        Affine([
            a * e + b * g,
            a * f + b * h,
            c * e + d * g,
            c * f + d * h,
            x * e + y * g + u,
            x * f + y * h + v,
        ])
    }

    fn inverse(&self) -> Affine {
        let [a, b, c, d, x, y] = self.0;
        let det = a * d - b * c;
        if det.abs() < 1e-12 {
            return Affine([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        }
        let [ia, ib, ic, id] = [d / det, -b / det, -c / det, a / det];
        Affine([ia, ib, ic, id, -(x * ia + y * ic), -(x * ib + y * id)])
    }

    fn apply(&self, [x, y]: [f32; 2]) -> [f32; 2] {
        let [a, b, c, d, u, v] = self.0;
        [x * a + y * c + u, x * b + y * d + v]
    }
}

impl Animation {
    /// Each bone's pose `seconds` in, at `rate`.
    fn at(&self, seconds: f32, rate: f32) -> Vec<Pose> {
        let length = self.frames as f32;
        let frame = (seconds * self.fps * rate).max(0.0);
        let frame = match self.mode {
            Mode::Loop => frame % length,
            Mode::Mirror => {
                let turn = frame % (2.0 * length);
                if turn > length {
                    2.0 * length - turn
                } else {
                    turn
                }
            }
            Mode::Single => frame.min(length),
        };
        let first = frame.floor();
        let along = frame - first;
        self.tracks
            .iter()
            .map(|track| {
                let last = track.len() - 1;
                let now = (first as usize).min(last);
                track[now].mix(&track[(now + 1).min(last)], along)
            })
            .collect()
    }
}

impl Puppet {
    /// Where each corner is `seconds` in with these animations playing, in order over the
    /// rest pose; the animations it doesn't have are let be.
    pub fn corners_at(&self, playing: &[Posed], seconds: f32) -> Vec<[f32; 2]> {
        let rest: Vec<Pose> = self.bones.iter().map(|bone| bone.rest).collect();
        let mut poses = rest.clone();
        for layer in playing {
            let Some(animation) = self
                .animations
                .iter()
                .find(|animation| animation.id == layer.animation)
            else {
                continue;
            };
            let blend = layer.blend.clamp(0.0, 1.0);
            for (bone, frame) in animation.at(seconds, layer.rate).iter().enumerate() {
                poses[bone] = if layer.additive {
                    poses[bone].add(frame, &rest[bone], blend)
                } else {
                    poses[bone].mix(frame, blend)
                };
            }
        }
        let placed = |poses: &[Pose]| {
            let mut world: Vec<Affine> = Vec::with_capacity(poses.len());
            for (bone, pose) in self.bones.iter().zip(poses) {
                let own = pose.affine();
                let placed = match bone.parent {
                    Some(parent) => own.then(&world[parent]),
                    None => own,
                };
                world.push(placed);
            }
            world
        };
        let skins: Vec<Affine> = placed(&rest)
            .iter()
            .zip(placed(&poses))
            .map(|(rest, now)| rest.inverse().then(&now))
            .collect();
        self.corners
            .iter()
            .map(|corner| {
                if skins.is_empty() {
                    return corner.position;
                }
                let total: f32 = corner.weights.iter().sum();
                if total <= 1e-6 {
                    return corner.position;
                }
                let mut moved = [0.0; 2];
                for (&bone, &weight) in corner.bones.iter().zip(&corner.weights) {
                    if weight == 0.0 {
                        continue;
                    }
                    let [x, y] = skins[usize::from(bone)].apply(corner.position);
                    moved[0] += x * weight / total;
                    moved[1] += y * weight / total;
                }
                moved
            })
            .collect()
    }
}
