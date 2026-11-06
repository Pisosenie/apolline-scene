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

#[test]
fn an_instance_scales_its_system_and_unknown_overrides_are_refused() {
    let folder = scene(
        "particle-overrides",
        r#"{"passes":[{"textures":["particle/flake"]}]}"#,
    );
    let scene = std::fs::read_to_string(folder.join("scene.json")).unwrap().replace(
        r#""origin":"5 10 0""#,
        r#""origin":"5 10 0","angles":"0 0 0.5","instanceoverride":{"id":3,"count":0.5,"rate":2,"colorn":"1 0 0"}"#,
    );
    std::fs::write(folder.join("scene.json"), &scene).unwrap();
    let prepared = Project::open(&folder, None).unwrap().prepare().unwrap();
    let system = prepared.layers[1].particles.as_ref().unwrap();
    assert_eq!((system.max, system.emitters[0].rate), (10, 20.0));
    assert_eq!(system.spawn.color.high, [1.0, 0.0, 0.0]);
    std::fs::write(
        folder.join("scene.json"),
        scene.replace(r#""rate":2"#, r#""wobble":2"#),
    )
    .unwrap();
    let refused = Project::open(&folder, None)
        .unwrap()
        .prepare()
        .err()
        .unwrap()
        .to_string();
    assert!(refused.contains("override wobble"), "{refused}");
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn worldspace_particles_draw_on_a_layer_that_stays_put() {
    let folder = scene(
        "particles-worldspace",
        r#"{"passes":[{"shader":"genericparticle","textures":["particle/flake"]}]}"#,
    );
    std::fs::write(
        folder.join("particles/snow.json"),
        r#"{"material":"materials/particle/flake.json","maxcount":20,"flags":1,
            "emitter":[{"name":"boxrandom","rate":10}],"renderer":[{"name":"sprite"}]}"#,
    )
    .unwrap();
    let project = Project::open(&folder, None).unwrap();
    assert!(
        project.prepare().unwrap().layers[1]
            .particles
            .as_ref()
            .unwrap()
            .world
    );
    // A timeline moving it leaves its particles behind, which isn't drawn.
    std::fs::write(
        folder.join("scene.json"),
        r#"{"general":{"orthogonalprojection":{"width":10,"height":10}},"objects":[
            {"name":"snow","particle":"particles/snow.json","origin":{"value":"5 10 0",
             "animation":{"c0":[{"frame":0,"value":0},{"frame":10,"value":5}],"options":{"fps":30,"length":10}}}}]}"#,
    )
    .unwrap();
    let refused = project.prepare().err().unwrap().to_string();
    assert!(
        refused.contains("worldspace particles on a moving layer"),
        "{refused}"
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn static_children_follow_their_parent_and_event_children_are_refused() {
    let folder = scene(
        "particle-children",
        r#"{"passes":[{"textures":["particle/flake"]}]}"#,
    );
    let write = |path: &str, text: &str| std::fs::write(folder.join(path), text).unwrap();
    let system = |children: &str| {
        format!(
            r#"{{"material":"materials/particle/flake.json","maxcount":20,
                "emitter":[{{"name":"boxrandom","rate":10}}],"renderer":[{{"name":"sprite"}}],
                "children":{children}}}"#
        )
    };
    write(
        "particles/snow.json",
        &system(
            r#"[{"name":"particles/spark.json","type":"static","origin":"10 0 0","scale":"2 2 2"}]"#,
        ),
    );
    write(
        "particles/spark.json",
        &system(r#"[{"name":"particles/dust.json","origin":"1 0 0","angles":"0 0 1"}]"#),
    );
    write("particles/dust.json", &system("[]"));
    let prepared = Project::open(&folder, None).unwrap().prepare().unwrap();
    let family: Vec<_> = prepared.layers[1..4]
        .iter()
        .map(|layer| (layer.object, layer.child.clone()))
        .collect();
    assert!(family.iter().all(|(object, _)| *object == 1));
    assert_eq!(family[0].1, None);
    let spark = family[1].1.as_ref().unwrap();
    assert_eq!((spark.origin, spark.scale), ([10.0, 0.0, 0.0], [2.0; 3]));
    // The dust sits from the spark at the spark's scale.
    let dust = family[2].1.as_ref().unwrap();
    assert_eq!(
        (dust.origin, dust.scale, dust.angles),
        ([12.0, 0.0, 0.0], [2.0; 3], [0.0, 0.0, 1.0])
    );
    assert!(prepared.layers[4].particles.is_none());

    write(
        "particles/dust.json",
        &system(r#"[{"name":"particles/dust.json"}]"#),
    );
    let refused = Project::open(&folder, None)
        .unwrap()
        .prepare()
        .err()
        .unwrap()
        .to_string();
    assert!(refused.contains("generations deep"), "{refused}");
    write(
        "particles/snow.json",
        &system(r#"[{"name":"particles/spark.json","type":"eventdeath"}]"#),
    );
    let refused = Project::open(&folder, None)
        .unwrap()
        .prepare()
        .err()
        .unwrap()
        .to_string();
    assert!(refused.contains("tied to particles"), "{refused}");
    std::fs::remove_dir_all(folder).unwrap();
}
