//! Smooth noise for turbulence: gradients on a whole-number lattice, eased between, the same
//! for a seed everywhere. Places are doubles so hours of time still move it smoothly.

/// Noise at `at`, about -1 to 1, changing smoothly from one whole number to the next.
pub fn noise(seed: u64, at: [f64; 3]) -> f32 {
    let cell = at.map(f64::floor);
    let within = [0, 1, 2].map(|axis| (at[axis] - cell[axis]) as f32);
    let eased = within.map(|t| t * t * t * (t * (t * 6.0 - 15.0) + 10.0));
    let mut total = 0.0;
    for corner in 0..8u32 {
        let side = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1].map(|bit| bit as f32);
        let lattice = [0, 1, 2].map(|axis| cell[axis] as i64 + side[axis] as i64);
        let gradient = gradient(seed, lattice);
        let mut dot = 0.0;
        let mut weight = 1.0;
        for axis in 0..3 {
            dot += gradient[axis] * (within[axis] - side[axis]);
            weight *= if side[axis] > 0.5 {
                eased[axis]
            } else {
                1.0 - eased[axis]
            };
        }
        total += dot * weight;
    }
    total.clamp(-1.0, 1.0)
}

/// Three noises, one along each axis, for a push in any direction. They share the lattice
/// and its weights, and each corner's one hash gives all three directions from its own bits:
/// turbulence asks for this per particle per step.
pub fn field(seed: u64, at: [f64; 3]) -> [f32; 3] {
    let cell = at.map(f64::floor);
    let within = [0, 1, 2].map(|axis| (at[axis] - cell[axis]) as f32);
    let eased = within.map(|t| t * t * t * (t * (t * 6.0 - 15.0) + 10.0));
    let mut total = [0.0f32; 3];
    for corner in 0..8u32 {
        let side = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
        let lattice = [0, 1, 2].map(|axis| cell[axis] as i64 + i64::from(side[axis]));
        let mut weight = 1.0;
        let mut offset = [0.0f32; 3];
        for axis in 0..3 {
            offset[axis] = within[axis] - side[axis] as f32;
            weight *= if side[axis] == 1 {
                eased[axis]
            } else {
                1.0 - eased[axis]
            };
        }
        let hash = hashed(seed, lattice);
        for (noise, total) in total.iter_mut().enumerate() {
            let gradient = EDGES[((hash >> (noise * 21)) & 0x1F_FFFF) as usize % 12];
            let dot: f32 = (0..3).map(|axis| gradient[axis] * offset[axis]).sum();
            *total += dot * weight;
        }
    }
    total.map(|value| value.clamp(-1.0, 1.0))
}

/// The twelve directions to a cube's edges.
const EDGES: [[f32; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

/// One of the twelve directions at a lattice point.
fn gradient(seed: u64, at: [i64; 3]) -> [f32; 3] {
    EDGES[(hashed(seed, at) % 12) as usize]
}

fn hashed(seed: u64, at: [i64; 3]) -> u64 {
    const MIXES: [u64; 3] = [
        0x9E37_79B9_7F4A_7C15,
        0xC2B2_AE3D_27D4_EB4F,
        0x1656_67B1_9E37_79F9,
    ];
    let mut z = seed;
    for (value, mix) in at.iter().zip(MIXES) {
        z ^= (*value as u64).wrapping_mul(mix);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    }
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_smooth_bounded_and_fixed_by_its_seed() {
        let at = |x: f64| noise(7, [x, 0.25, 0.5]);
        assert_eq!(at(1.3), at(1.3));
        assert_ne!(at(1.3), noise(8, [1.3, 0.25, 0.5]));
        let mut spread: f32 = 0.0;
        for step in 0..2000 {
            let x = step as f64 * 0.01;
            let (here, next) = (at(x), at(x + 0.01));
            assert!(here.abs() <= 1.0);
            assert!((here - next).abs() < 0.05, "{x}");
            spread = spread.max(here.abs());
        }
        assert!(spread > 0.2);
        // Far along, it still moves smoothly.
        let far = 3_600.0 * 50.0;
        assert!((at(far) - at(far + 0.001)).abs() < 0.01);
        assert_ne!(at(far), at(far + 0.5));
    }
}
