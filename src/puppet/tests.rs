use super::*;

fn floats(out: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

fn words(out: &mut Vec<u8>, values: &[u32]) {
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
}

/// A two-bone arm, the second bone 100 right of the first, and a mesh whose corners hang from
/// one bone each; its animation turns the second bone a quarter turn over a second.
fn arm() -> Vec<u8> {
    let mut out = b"MDLV0016\0".to_vec();
    words(&mut out, &[0x0180_0009, 1, 1]);
    out.extend_from_slice(b"materials/arm.json\0");
    words(&mut out, &[0, 0x0180_0009, 3 * 52]);
    for (position, bone, uv) in [
        ([0.0, 0.0], 0, [0.0, 0.0]),
        ([100.0, 0.0], 1, [0.5, 0.0]),
        ([200.0, 0.0], 1, [1.0, 1.0]),
    ] {
        floats(&mut out, &[position[0], position[1], 0.0]);
        words(&mut out, &[bone, 0, 0, 0]);
        floats(&mut out, &[1.0, 0.0, 0.0, 0.0]);
        floats(&mut out, &uv);
    }
    words(&mut out, &[6]);
    for index in [0u16, 1, 2] {
        out.extend_from_slice(&index.to_le_bytes());
    }
    let skeleton = out.len();
    out.extend_from_slice(b"MDLS0002\0");
    words(&mut out, &[0, 2]);
    for (parent, x) in [(u32::MAX, 0.0), (0, 100.0)] {
        out.push(0);
        words(&mut out, &[1, parent, 64]);
        floats(
            &mut out,
            &[
                1.0, 0., 0., 0., 0., 1.0, 0., 0., 0., 0., 1.0, 0., x, 0., 0., 1.0,
            ],
        );
        out.extend_from_slice(b"{\"tp\":null}\0");
    }
    // What the editor draws the bones with, which nothing here reads.
    out.extend_from_slice(&[0; 13]);
    let end = out.len() as u32;
    out[skeleton + 9..skeleton + 13].copy_from_slice(&end.to_le_bytes());
    out.extend_from_slice(b"MDLA0003\0");
    words(&mut out, &[0, 1, 7, 0]);
    out.extend_from_slice("Animation 1\0loop\0".as_bytes());
    floats(&mut out, &[2.0]);
    words(&mut out, &[2, 0, 2]);
    for (bone, x) in [(0, 0.0), (1, 100.0)] {
        words(&mut out, &[0, 3 * 36]);
        for frame in 0..3 {
            let angle = if bone == 1 && frame == 2 {
                std::f32::consts::FRAC_PI_2
            } else {
                0.0
            };
            floats(&mut out, &[x, 0.0, 0.0, 0.0, 0.0, angle, 1.0, 1.0, 1.0]);
        }
    }
    out.extend_from_slice(&[0; 9]);
    out
}

#[test]
fn a_puppet_reads_its_mesh_skeleton_and_animation() {
    let puppet = Puppet::parse(&arm()).unwrap();
    assert_eq!(puppet.corners.len(), 3);
    assert_eq!(puppet.corners[2].uv, [1.0, 1.0]);
    assert_eq!(puppet.triangles, [[0, 1, 2]]);
    assert_eq!(puppet.bones[1].parent, Some(0));
    assert_eq!(puppet.bones[1].rest.position, [100.0, 0.0]);
    let animation = &puppet.animations[0];
    assert_eq!(
        (animation.id, animation.frames, animation.mode),
        (7, 2, Mode::Loop)
    );
    assert!(Puppet::parse(&arm()[..200]).is_err());
}

#[test]
fn corners_move_with_the_bones_they_hang_from() {
    let puppet = Puppet::parse(&arm()).unwrap();
    let playing = Posed {
        animation: 7,
        rate: 1.0,
        blend: 1.0,
        additive: false,
    };
    let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-2 && (a[1] - b[1]).abs() < 1e-2;
    // At rest, and at the start, nothing has moved.
    assert_eq!(
        puppet.corners_at(&[], 0.5),
        [[0.0, 0.0], [100.0, 0.0], [200.0, 0.0]]
    );
    let start = puppet.corners_at(&[playing], 0.0);
    assert!(near(start[2], [200.0, 0.0]));
    // A second in, at frame 2, the second bone has turned a quarter: its far corner points up.
    let turned = puppet.corners_at(&[playing], 0.999_99);
    assert!(near(turned[0], [0.0, 0.0]) && near(turned[1], [100.0, 0.0]));
    assert!(near(turned[2], [100.0, 100.0]), "{:?}", turned[2]);
    // Half blended, it's turned an eighth.
    let half = puppet.corners_at(
        &[Posed {
            blend: 0.5,
            ..playing
        }],
        0.999_99,
    );
    let eighth = std::f32::consts::FRAC_PI_4;
    assert!(near(
        half[2],
        [100.0 + 100.0 * eighth.cos(), 100.0 * eighth.sin()]
    ));
    // At half the rate it's only halfway there.
    let slow = puppet.corners_at(
        &[Posed {
            rate: 0.5,
            ..playing
        }],
        0.999_99,
    );
    assert!(near(slow[2], [200.0, 0.0]) || slow[2][1] < 1.0);
}
