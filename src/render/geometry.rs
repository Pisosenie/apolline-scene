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
