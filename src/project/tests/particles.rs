use super::{folder, texture};
use crate::Project;

fn scene(name: &str, material: &str) -> std::path::PathBuf {
    let folder = folder(name);
    std::fs::create_dir_all(folder.join("materials/particle")).unwrap();
    std::fs::create_dir_all(folder.join("particles")).unwrap();
    let write = |path: &str, text: &str| std::fs::write(folder.join(path), text).unwrap();
    write("project.json", "{}");
    write(
        "scene.json",
        r#"{"general":{"orthogonalprojection":{"width":10,"height":10}},"objects":[
            {"name":"sky","image":"model.json"},
            {"name":"snow","particle":"particles/snow.json","origin":"5 10 0"},
            {"name":"front","image":"model.json"}]}"#,
    );
    write("model.json", r#"{"material":"material.json"}"#);
    write("material.json", r#"{"passes":[{"textures":["picture"]}]}"#);
    write(
        "particles/snow.json",
        r#"{"material":"materials/particle/flake.json","maxcount":20,
            "emitter":[{"name":"boxrandom","rate":10}],"renderer":[{"name":"sprite"}]}"#,
    );
    write("materials/particle/flake.json", material);
    std::fs::write(folder.join("materials/picture.tex"), texture(1, 1)).unwrap();
    std::fs::write(folder.join("materials/particle/flake.tex"), texture(1, 1)).unwrap();
    folder
}

#[test]
fn particle_systems_are_layers_in_scene_order() {
    let folder = scene(
        "particles",
        r#"{"passes":[{"shader":"genericparticle","textures":["particle/flake"],"blending":"additive","combos":{"SPRITESHEET":0}}]}"#,
    );
    let project = Project::open(&folder, None).unwrap();
    let prepared = project.prepare().unwrap();
    let kinds: Vec<bool> = prepared
        .layers
        .iter()
        .map(|layer| layer.particles.is_some())
        .collect();
    assert_eq!(kinds, [false, true, false]);
    assert_eq!(prepared.layers[1].blending, crate::Blending::Additive);
    assert_eq!(prepared.layers[1].image.origin, [5.0, 10.0, 0.0]);
    assert!(project.prepare_still().is_err());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn particle_materials_beyond_the_plain_sprite_are_refused() {
    for (at, (material, expected)) in [
        (r#"{"passes":[{"shader":"particles/custom","textures":["particle/flake"]}]}"#, "their own shader"),
        (r#"{"passes":[{"shader":"genericparticle","textures":["particle/flake"],"combos":{"SPRITESHEET":1}}]}"#, "shader options"),
        (r#"{"passes":[{"textures":["particle/flake"],"blending":"multiply"}]}"#, "blending"),
        (r#"{"passes":[{"textures":["particle/flake",null,"x"]}]}"#, "one texture"),
    ]
    .into_iter()
    .enumerate()
    {
        let folder = scene(&format!("particle-refusal-{at}"), material);
        let refused = Project::open(&folder, None).unwrap().prepare().err().unwrap().to_string();
        assert!(refused.contains(expected), "{expected}: {refused}");
        std::fs::remove_dir_all(folder).unwrap();
    }
}
