//! Where layers land on the target: the scene's own pixels, with y up, fitted to the target
//! the way a wallpaper is, and the parallax easing after the pointer.

/// A 2D affine map from a layer's unit square to clip space, as two rows:
/// x' = r0·(x, y, 1), y' = r1·(x, y, 1).
pub type Affine = [[f32; 3]; 2];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    #[default]
    Cover,
    Contain,
}

/// Where a layer's unit square, x and y in -0.5..0.5, lands in clip space: `size` in scene
/// pixels centred on `origin`, turned by `angle`, in a `scene`-sized scene that covers a
/// `target`-sized framebuffer.
pub fn place(
    scene: (f32, f32),
    target: (u32, u32),
    origin: [f32; 2],
    size: (f32, f32),
    angle: f32,
) -> Affine {
    place_fitted(scene, target, origin, size, angle, Fit::Cover)
}

pub fn place_fitted(
    scene: (f32, f32),
    target: (u32, u32),
    origin: [f32; 2],
    size: (f32, f32),
    angle: f32,
    fit: Fit,
) -> Affine {
    let (sw, sh) = scene;
    let (tw, th) = (target.0.max(1) as f32, target.1.max(1) as f32);
    let fit = match fit {
        Fit::Cover => (tw / sw).max(th / sh),
        Fit::Contain => (tw / sw).min(th / sh),
    };
    let (kx, ky) = (2.0 * fit / tw, 2.0 * fit / th);
    let (sin, cos) = angle.sin_cos();
    let (w, h) = size;
    [
        [kx * cos * w, -kx * sin * h, kx * (origin[0] - sw / 2.0)],
        [ky * sin * w, ky * cos * h, ky * (origin[1] - sh / 2.0)],
    ]
}

/// Where in the scene, `scene` big with y up, the pointer is when its whole is placed by
/// `affine`; the pointer runs 0 to 1 across the screen from the top left.
pub(super) fn pointed(affine: &Affine, scene: (f32, f32), pointer: [f32; 2]) -> [f32; 2] {
    let [[a, b, x], [c, d, y]] = *affine;
    let ndc = [pointer[0] * 2.0 - 1.0 - x, 1.0 - pointer[1] * 2.0 - y];
    let det = a * d - b * c;
    if det.abs() < 1e-12 {
        return [scene.0 / 2.0, scene.1 / 2.0];
    }
    let unit = [
        (d * ndc[0] - b * ndc[1]) / det,
        (a * ndc[1] - c * ndc[0]) / det,
    ];
    [(unit[0] + 0.5) * scene.0, (unit[1] + 0.5) * scene.1]
}

/// Where a shaking camera has moved `time` seconds in, in shares of the scene's size: a smooth
/// wander a two-hundredth of the scene across per unit of amplitude, quicker with speed and
/// jitterier with roughness.
pub(super) fn shaken(shake: crate::scene::Shake, time: f32) -> [f32; 2] {
    let t = time * shake.speed;
    let wander = |a: f32, b: f32, c: f32| {
        (t * a).sin() + 0.5 * (t * b + 1.7).sin() + 0.25 * shake.roughness * (t * c + 0.3).sin()
    };
    let reach = shake.amplitude * 0.005 / (1.5 + 0.25 * shake.roughness);
    [reach * wander(1.3, 2.9, 7.1), reach * wander(1.7, 3.7, 8.3)]
}

/// `seconds` of the way from `eye` to the pointer, when the whole way takes `delay`.
pub(super) fn toward(eye: [f32; 2], pointer: [f32; 2], seconds: f32, delay: f32) -> [f32; 2] {
    let reach = if delay > 0.0 {
        (seconds / delay).clamp(0.0, 1.0)
    } else {
        1.0
    };
    std::array::from_fn(|at| eye[at] + (pointer[at].clamp(-1.0, 1.0) - eye[at]) * reach)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_is_found_in_the_scene_however_it_is_fitted_and_zoomed() {
        let scene = (1920.0, 1080.0);
        let whole = place_fitted(scene, (1280, 720), [960.0, 540.0], scene, 0.0, Fit::Cover);
        assert_eq!(pointed(&whole, scene, [0.5, 0.5]), [960.0, 540.0]);
        // Top left on screen is the scene's top left, which is up its y.
        assert_eq!(pointed(&whole, scene, [0.0, 0.0]), [0.0, 1080.0]);
        let zoomed = whole.map(|row| row.map(|value| value * 2.0));
        assert_eq!(pointed(&zoomed, scene, [0.0, 0.0]), [480.0, 810.0]);
    }

    #[test]
    fn a_shaking_camera_wanders_no_further_than_its_amplitude() {
        let shake = crate::scene::Shake {
            amplitude: 0.5,
            roughness: 1.0,
            speed: 3.0,
        };
        let places: Vec<[f32; 2]> = (0..600).map(|at| shaken(shake, at as f32 / 60.0)).collect();
        assert!(
            places
                .iter()
                .flatten()
                .all(|share| share.abs() <= 0.5 * 0.005 + 1e-6)
        );
        assert!(places.windows(2).any(|pair| pair[0] != pair[1]));
    }

    fn apply(affine: Affine, x: f32, y: f32) -> (f32, f32) {
        let row = |r: [f32; 3]| r[0] * x + r[1] * y + r[2];
        (row(affine[0]), row(affine[1]))
    }

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
    }

    #[test]
    fn the_parallax_eases_towards_the_pointer_and_stays_in_range() {
        assert_eq!(toward([0.0, 0.0], [1.0, -1.0], 0.05, 0.1), [0.5, -0.5]);
        assert_eq!(toward([0.0, 0.0], [3.0, 0.0], 1.0, 0.1), [1.0, 0.0]);
        assert_eq!(toward([0.2, 0.0], [1.0, 0.0], 0.01, 0.0), [1.0, 0.0]);
    }

    #[test]
    fn a_layer_the_size_of_the_scene_fills_a_target_of_its_shape() {
        let affine = place(
            (1920.0, 1080.0),
            (3840, 2160),
            [960.0, 540.0],
            (1920.0, 1080.0),
            0.0,
        );
        assert!(close(apply(affine, -0.5, -0.5), (-1.0, -1.0)));
        assert!(close(apply(affine, 0.5, 0.5), (1.0, 1.0)));
    }

    #[test]
    fn a_narrower_target_crops_the_sides_and_keeps_the_middle() {
        // A 16:9 scene on a square target fills its height and spills past both sides.
        let affine = place(
            (1600.0, 900.0),
            (900, 900),
            [800.0, 450.0],
            (1600.0, 900.0),
            0.0,
        );
        let (right, top) = apply(affine, 0.5, 0.5);
        assert!((top - 1.0).abs() < 1e-5);
        assert!((right - 16.0 / 9.0).abs() < 1e-5);
    }

    #[test]
    fn origins_put_layers_in_place_and_angles_turn_them() {
        // A 100×100 layer in the top right corner of a 1000×1000 scene.
        let affine = place(
            (1000.0, 1000.0),
            (1000, 1000),
            [950.0, 950.0],
            (100.0, 100.0),
            0.0,
        );
        assert!(close(apply(affine, 0.5, 0.5), (1.0, 1.0)));
        assert!(close(apply(affine, -0.5, -0.5), (0.8, 0.8)));
        // A quarter turn takes the right edge's middle to the top's.
        let turned = place(
            (1000.0, 1000.0),
            (1000, 1000),
            [500.0, 500.0],
            (100.0, 100.0),
            std::f32::consts::FRAC_PI_2,
        );
        assert!(close(apply(turned, 0.5, 0.0), (0.0, 0.1)));
    }
}
