use super::*;
use crate::pkg;
use std::path::PathBuf;

mod effects;
mod particles;

fn texture(width: u32, height: u32) -> Vec<u8> {
    texture_format(width, height, 0, 4)
}

fn texture_format(width: u32, height: u32, format: u32, channels: u32) -> Vec<u8> {
    let mut out = b"TEXV0005\0TEXI0001\0".to_vec();
    for n in [format, 0, width, height, width, height, 0] {
        out.extend(u32::to_le_bytes(n));
    }
    out.extend(b"TEXB0002\0");
    let pixels = vec![200u8; (width * height * channels) as usize];
    for n in [1, 1, width, height, 0, 0, pixels.len() as u32] {
        out.extend(u32::to_le_bytes(n));
    }
    out.extend(pixels);
    out
}

fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("apolline-scene-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

#[test]
fn preparation_diagnostics_are_bounded_when_many_layers_fail() {
    let folder = folder("diagnostics");
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    let project = Project::open(&folder, None).unwrap();
    let object = serde_json::json!({"image":"absent.json","name":"x".repeat(10000)});
    let scene = Scene::from_value(&serde_json::json!({"objects":vec![object;100]})).unwrap();
    let (layers, skipped) = project.layers(&scene);
    assert!(layers.is_empty());
    assert_eq!(skipped.len(), 65);
    assert!(skipped.last().unwrap().contains("36 additional"));
    assert!(skipped.join("; ").len() < 128 * 1024);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn static_fragment_materials_prepare_with_bound_properties_and_shared_sources() {
    let folder = folder("fragment");
    std::fs::create_dir_all(folder.join("shaders")).unwrap();
    std::fs::create_dir_all(folder.join("materials")).unwrap();
    std::fs::write(folder.join("project.json"), r#"{"general":{"properties":{"amount":{"type":"slider","value":1,"min":0,"max":1,"fraction":true}}}}"#).unwrap();
    std::fs::write(folder.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":10,"height":10}},"objects":[{"image":"model.json"},{"image":"other.json"}]}"#).unwrap();
    for model in ["model.json", "other.json"] {
        std::fs::write(folder.join(model), r#"{"material":"material.json"}"#).unwrap();
    }
    std::fs::write(folder.join("material.json"), r#"{"passes":[{"shader":"demo","textures":["same"],"constantshadervalues":{"amount":{"user":"amount","value":1}}}]}"#).unwrap();
    std::fs::write(folder.join("shaders/demo.frag"), "uniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\nvoid main() { gl_FragColor = vec4(u_Amount); }").unwrap();
    std::fs::write(folder.join("materials/same.tex"), texture(1, 1)).unwrap();
    let mut project = Project::open(&folder, None).unwrap();
    project
        .set_properties(&Values::from([("amount".into(), serde_json::json!(0.25))]))
        .unwrap();
    let prepared = project.prepare_still().unwrap();
    let first = prepared.layers[0].fragment.as_ref().unwrap();
    assert_eq!(first.constants[0].1.floats()[0], 0.25);
    assert!(Arc::ptr_eq(
        first,
        prepared.layers[1].fragment.as_ref().unwrap()
    ));
    std::fs::write(
        folder.join("materials/same.tex"),
        texture_format(1, 1, 9, 1),
    )
    .unwrap();
    assert_eq!(
        project.prepare_still().unwrap().layers[0].texture.images[0][0].channels,
        crate::tex::Channels::R
    );

    std::fs::write(folder.join("shaders/demo.frag"), "uniform float g_Time;\nuniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\nvoid main() { gl_FragColor = vec4(u_Amount + g_Time); }").unwrap();
    assert!(
        project
            .prepare_still()
            .err()
            .unwrap()
            .to_string()
            .contains("time-dependent")
    );
    assert!(
        project.prepare().unwrap().layers[0]
            .fragment
            .as_ref()
            .unwrap()
            .animated
    );

    // A material's own vertex shader is prepared with its fragment, and the fragment can read
    // only what it writes.
    std::fs::write(folder.join("shaders/demo.vert"), "void main() {}").unwrap();
    assert!(
        project.prepare().unwrap().layers[0]
            .fragment
            .as_ref()
            .unwrap()
            .vertex
            .is_some()
    );
    std::fs::write(
        folder.join("shaders/demo.frag"),
        "uniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\nvarying vec4 v_TexCoord;\nvoid main() { gl_FragColor = vec4(v_TexCoord.zw, u_Amount, 1.0); }",
    )
    .unwrap();
    let refused = project.prepare().err().map(|err| err.to_string());
    assert!(
        refused
            .as_deref()
            .is_some_and(|err| err.contains("vertex shader")),
        "{refused:?}"
    );
    std::fs::write(
        folder.join("shaders/demo.vert"),
        "attribute vec3 a_Position;\nattribute vec2 a_TexCoord;\nuniform mat4 g_ModelViewProjectionMatrix;\nuniform float u_Amount; // {\"material\":\"amount\",\"default\":1}\nvarying vec4 v_TexCoord;\nvoid main() { gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix); v_TexCoord = vec4(a_TexCoord, a_TexCoord * u_Amount); }",
    )
    .unwrap();
    assert!(project.prepare().is_ok());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn fragment_inputs_share_decodes_and_all_count_toward_the_budget() {
    let folder = folder("fragment-inputs");
    std::fs::create_dir_all(folder.join("shaders")).unwrap();
    std::fs::create_dir_all(folder.join("materials")).unwrap();
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    std::fs::write(folder.join("shaders/demo.frag"), "uniform sampler2D g_Texture0;\nuniform sampler2D g_Texture1;\nvarying vec2 v_TexCoord;\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord) * texSample2D(g_Texture1, v_TexCoord); }").unwrap();
    for name in ["same", "other"] {
        std::fs::write(folder.join(format!("materials/{name}.tex")), texture(4, 4)).unwrap();
    }
    let project = Project::open(&folder, None).unwrap();
    let scene = Scene::parse(r#"{"objects":[{"image":"model.json"}]}"#).unwrap();
    let material = |names: &[&str], shader: &str| {
        std::fs::write(
            folder.join("material.json"),
            serde_json::json!({
                "passes":[{"shader":shader,"textures":names}]
            })
            .to_string(),
        )
        .unwrap();
    };
    material(&["same", "same"], "demo");
    let (layers, skipped) = project.layers_with_budget(&scene, 64);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(layers[0].secondary.len(), 1);
    assert!(Arc::ptr_eq(
        &layers[0].texture,
        &layers[0].secondary[0].texture
    ));
    material(&["same", "other"], "demo");
    let (layers, skipped) = project.layers_with_budget(&scene, 128);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert!(!Arc::ptr_eq(
        &layers[0].texture,
        &layers[0].secondary[0].texture
    ));
    let (layers, skipped) = project.layers_with_budget(&scene, 127);
    assert!(layers.is_empty());
    assert_eq!(skipped.len(), 1);
    for (format, channels) in [(9, 1), (8, 2)] {
        std::fs::write(
            folder.join("materials/other.tex"),
            texture_format(4, 4, format, channels),
        )
        .unwrap();
        let budget = 64 + 16 * channels as usize;
        let (layers, skipped) = project.layers_with_budget(&scene, budget);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(
            layers[0].secondary[0].texture.images[0][0].pixels.len(),
            16 * channels as usize
        );
        assert!(project.layers_with_budget(&scene, budget - 1).0.is_empty());
    }
    material(&["absent"], "demo");
    let (_, skipped) = project.layers(&scene);
    assert!(skipped[0].contains("required texture slots"));
    for (names, shader) in [(vec!["same"; 9], "demo"), (vec!["same"; 2], "genericimage")] {
        material(&names, shader);
        let (layers, skipped) = project.layers(&scene);
        assert!(layers.is_empty());
        assert_eq!(skipped.len(), 1);
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn repeated_models_resolve_once_and_the_cache_does_not_escape_preparation() {
    let folder = folder("model-cache");
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    std::fs::create_dir(folder.join("materials")).unwrap();
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"textures":["same"],"blending":"additive"}]}"#,
    )
    .unwrap();
    std::fs::write(folder.join("materials/same.tex"), texture(4, 4)).unwrap();
    let project = Project::open(&folder, None).unwrap();
    let scene = Scene::parse(
        r#"{"objects":[{"image":"model.json"},{"image":"model.json","origin":"5 6 0"}]}"#,
    )
    .unwrap();
    let images: Vec<_> = scene.images().collect();
    let (mut models, mut textures) = (HashMap::new(), material::Resources::default());
    let mut remaining = 64;
    let first = project
        .layer(images[0], &mut models, &mut textures, &mut remaining, false)
        .unwrap();
    std::fs::remove_file(folder.join("model.json")).unwrap();
    std::fs::remove_file(folder.join("material.json")).unwrap();
    let second = project
        .layer(images[1], &mut models, &mut textures, &mut remaining, false)
        .unwrap();
    assert!(Arc::ptr_eq(&first.texture, &second.texture));
    assert_eq!(second.image.origin, [5.0, 6.0, 0.0]);
    assert_eq!(second.blending, Blending::Additive);
    assert_eq!(remaining, 0);
    assert_eq!(models.len(), 1);
    let (layers, skipped) = project.layers(&scene);
    assert!(layers.is_empty());
    assert_eq!(skipped.len(), 2);
    let mut failed = HashMap::new();
    assert!(
        project
            .layer(images[0], &mut failed, &mut textures, &mut remaining, false)
            .is_err()
    );
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"textures":["same"]}]}"#,
    )
    .unwrap();
    assert!(
        project
            .layer(images[0], &mut failed, &mut textures, &mut remaining, false)
            .is_err()
    );
    let (layers, skipped) = project.layers(&scene);
    assert_eq!(layers.len(), 2);
    assert!(skipped.is_empty());
    assert!(!relative(&"x".repeat(4097)));
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn resource_errors_keep_the_reason_instead_of_becoming_missing_files() {
    let folder = folder("errors");
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    std::fs::create_dir(folder.join("bad.json")).unwrap();
    let project = Project::open(&folder, None).unwrap();
    assert!(
        project
            .json("bad.json")
            .unwrap_err()
            .to_string()
            .contains("regular file")
    );
    assert!(
        project
            .json("../outside.json")
            .unwrap_err()
            .to_string()
            .contains("inside the project")
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn property_edits_reach_scene_bindings_and_invalid_edits_are_atomic() {
    let folder = folder("properties");
    std::fs::write(folder.join("project.json"), r#"{"general":{"properties":{"show":{"type":"bool","value":true},"opacity":{"type":"slider","min":0,"max":1,"fraction":true,"value":0.8}}}}"#).unwrap();
    std::fs::write(folder.join("scene.json"), r#"{"objects":[{"image":"model.json","visible":{"user":"show","value":false},"alpha":{"user":"opacity","value":1}}]}"#).unwrap();
    let mut project = Project::open(&folder, None).unwrap();
    assert!(project.scene().unwrap().images().next().unwrap().visible);
    let edits = Values::from([
        ("show".into(), serde_json::json!(false)),
        ("opacity".into(), serde_json::json!(0.25)),
    ]);
    project.set_properties(&edits).unwrap();
    let scene = project.scene().unwrap();
    let layer = scene.images().next().unwrap();
    assert!(!layer.visible);
    assert_eq!(layer.alpha, 0.25);
    // A saved edit that no longer fits, or names a property that's gone, gives way to the
    // default instead of keeping the scene from loading.
    project
        .set_properties(&Values::from([
            ("opacity".into(), serde_json::json!(2)),
            ("removed".into(), serde_json::json!(true)),
        ]))
        .unwrap();
    assert_eq!(project.scene().unwrap().images().next().unwrap().alpha, 0.8);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn layers_resolve_through_the_package_and_the_assets() {
    let folder = folder("project");
    let assets = folder.join("assets");
    std::fs::create_dir_all(assets.join("materials")).unwrap();
    std::fs::write(assets.join("materials/shared.tex"), texture(2, 2)).unwrap();
    std::fs::write(
        folder.join("project.json"),
        r#"{"title": " Rainy roof ", "type": "scene", "file": "scene.json"}"#,
    )
    .unwrap();
    let scene = br#"{"general": {"orthogonalprojection": {"width": 100, "height": 50}},
        "objects": [
            {"name": "roof", "image": "models/roof.json", "origin": "50 25 0"},
            {"name": "glow", "image": "models/glow.json"},
            {"name": "lost", "image": "models/lost.json"}
        ]}"#;
    let tex = texture(4, 2);
    std::fs::write(
        folder.join("scene.pkg"),
        pkg::build(&[
            ("scene.json", scene),
            (
                "models/roof.json",
                br#"{"material": "materials/roof.json", "fullscreen": true}"#,
            ),
            (
                "materials/roof.json",
                br#"{"passes": [{"textures": ["roof"], "blending": "normal"}]}"#,
            ),
            ("materials/roof.tex", &tex),
            (
                "models/glow.json",
                br#"{"material": "materials/glow.json"}"#,
            ),
            (
                "materials/glow.json",
                br#"{"passes": [{"textures": ["shared"], "blending": "additive"}]}"#,
            ),
        ]),
    )
    .unwrap();
    let project = Project::open(&folder, Some(&assets)).unwrap();
    assert_eq!(project.title, "Rainy roof");
    let scene = project.scene().unwrap();
    let (layers, skipped) = project.layers(&scene);
    assert_eq!(layers.len(), 2);
    assert_eq!(
        (layers[0].blending, layers[0].fullscreen),
        (Blending::Opaque, true)
    );
    assert_eq!(layers[0].texture.width, 4);
    assert_eq!(layers[1].blending, Blending::Additive);
    assert_eq!(layers[1].texture.width, 2);
    assert_eq!(skipped.len(), 1);
    assert!(skipped[0].starts_with("lost: "));
    assert!(project.file("../project.json").is_none());
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn only_scenes_open() {
    let folder = folder("video");
    std::fs::write(
        folder.join("project.json"),
        r#"{"type": "video", "file": "a.mp4"}"#,
    )
    .unwrap();
    assert!(Project::open(&folder, None).is_err());
    std::fs::remove_dir_all(&folder).unwrap();
}

#[test]
fn repeated_layers_share_pixels_and_hidden_layers_need_no_files() {
    let folder = folder("shared");
    std::fs::write(folder.join("project.json"), r#"{"type":"scene"}"#).unwrap();
    std::fs::create_dir(folder.join("materials")).unwrap();
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"textures":["same"]}]}"#,
    )
    .unwrap();
    std::fs::write(folder.join("materials/same.tex"), texture(4, 4)).unwrap();
    let project = Project::open(&folder, None).unwrap();
    let scene = Scene::parse(
        r#"{"objects":[
        {"image":"model.json"}, {"image":"model.json"},
        {"image":"absent.json","visible":false}
    ]}"#,
    )
    .unwrap();
    let (layers, skipped) = project.layers(&scene);
    assert!(skipped.is_empty());
    assert_eq!(layers.len(), 2);
    assert!(Arc::ptr_eq(&layers[0].texture, &layers[1].texture));
    let (shared, skipped) = project.layers_with_budget(&scene, 64);
    assert_eq!(shared.len(), 2);
    assert!(skipped.is_empty());
    let (too_large, skipped) = project.layers_with_budget(&scene, 63);
    assert!(too_large.is_empty());
    assert_eq!(skipped.len(), 2);
    std::fs::write(folder.join("materials/same.tex"), texture(2, 2)).unwrap();
    let (reloaded, _) = project.layers(&scene);
    assert_eq!(reloaded[0].texture.width, 2);
    assert_eq!(layers[0].texture.width, 4);
    std::fs::write(folder.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":100,"height":100}},"objects":[{"image":"model.json"}]}"#).unwrap();
    assert_eq!(project.prepare_still().unwrap().layers.len(), 1);
    for feature in [
        r#"{"particle":"rain.json"}"#,
        r#"{"image":"model.json","origin":{"script":"export function update(value) { return value; }","value":"0 0 0"}}"#,
    ] {
        std::fs::write(folder.join("scene.json"), format!(r#"{{"general":{{"orthogonalprojection":{{"width":100,"height":100}}}},"objects":[{feature}]}}"#)).unwrap();
        assert!(project.prepare_still().is_err());
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn a_scripted_scene_keeps_hidden_layers_for_its_scripts_to_show() {
    let folder = folder("scripted");
    std::fs::write(
        folder.join("project.json"),
        r#"{"general":{"properties":{"speed":{"type":"slider","min":0,"max":5,"value":2}}}}"#,
    )
    .unwrap();
    std::fs::create_dir(folder.join("materials")).unwrap();
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"textures":["one"]}]}"#,
    )
    .unwrap();
    std::fs::write(folder.join("materials/one.tex"), texture(4, 4)).unwrap();
    let show = serde_json::to_string(
        "export function update(value) { thisScene.getLayer('hidden').visible = true; return engine.userProperties.speed; }",
    )
    .unwrap();
    std::fs::write(
        folder.join("scene.json"),
        format!(
            r#"{{"general":{{"orthogonalprojection":{{"width":100,"height":100}}}},"objects":[
                {{"name":"shown","image":"model.json","alpha":{{"script":{show},"value":1}}}},
                {{"name":"hidden","image":"model.json","visible":false}}]}}"#
        ),
    )
    .unwrap();
    let project = Project::open(&folder, None).unwrap();
    let prepared = project.prepare().unwrap();
    assert_eq!(
        prepared
            .layers
            .iter()
            .map(|layer| layer.object)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    let mut stage = prepared.stage().unwrap().unwrap();
    let poses = stage.tick(crate::Moment {
        time: 0.1,
        delta: 0.1,
        daytime: 0.0,
        screen: (100, 100),
        spectrum: None,
    });
    assert_eq!((poses[0].alpha, poses[1].visible), (2.0, true));
    let broken = serde_json::to_string(
        "export function update(value) { return thisLayer.getAnimation().frame; }",
    )
    .unwrap();
    std::fs::write(
        folder.join("scene.json"),
        format!(r#"{{"general":{{"orthogonalprojection":{{"width":100,"height":100}}}},"objects":[{{"name":"puppet","image":"model.json","alpha":{{"script":{broken},"value":1}}}}]}}"#),
    )
    .unwrap();
    let error = project.prepare().err().unwrap().to_string();
    assert!(
        error.contains("puppet's alpha") && error.contains("getAnimation"),
        "{error}"
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn project_files_cannot_escape_their_roots() {
    let folder = folder("paths");
    let project_dir = folder.join("project");
    std::fs::create_dir(&project_dir).unwrap();
    std::fs::write(folder.join("outside"), "outside").unwrap();
    std::fs::write(project_dir.join("project.json"), "{}").unwrap();
    let project = Project::open(&project_dir, None).unwrap();
    for name in [
        "",
        "../outside",
        "/etc/passwd",
        "a/../../outside",
        "C:\\outside",
        "..\\outside",
    ] {
        assert!(project.file(name).is_none(), "{name}");
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(folder.join("outside"), project_dir.join("link")).unwrap();
        assert!(project.file("link").is_none());
    }
    assert_eq!(project.file("./project.json").unwrap().as_ref(), b"{}");
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn snapshots_allow_neutral_parallax_but_refuse_texture_animation_and_scripts() {
    let folder = folder("snapshot");
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    std::fs::write(folder.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":10,"height":10},"cameraparallax":true},"objects":[]}"#).unwrap();
    let project = Project::open(&folder, None).unwrap();
    assert!(project.prepare().is_err());
    assert!(project.prepare_snapshot().is_ok());
    std::fs::write(folder.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":10,"height":10},"zoom":{"script":"time","value":1}}}"#).unwrap();
    assert!(project.prepare_snapshot().is_err());
    std::fs::create_dir(folder.join("materials")).unwrap();
    std::fs::write(folder.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":10,"height":10}},"objects":[{"image":"model.json"}]}"#).unwrap();
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"textures":["animated"]}]}"#,
    )
    .unwrap();
    let mut animated = texture(1, 1);
    animated[22..26].copy_from_slice(&crate::tex::ANIMATED.to_le_bytes());
    std::fs::write(folder.join("materials/animated.tex"), animated).unwrap();
    assert!(
        project
            .prepare_snapshot()
            .err()
            .unwrap()
            .to_string()
            .contains("animated")
    );
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn optional_sparse_slots_only_decode_bound_resources_and_charge_their_bytes() {
    let folder = folder("optional-inputs");
    std::fs::create_dir(folder.join("materials")).unwrap();
    std::fs::create_dir(folder.join("shaders")).unwrap();
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    std::fs::write(folder.join("materials/same.tex"), texture(4, 4)).unwrap();
    std::fs::write(folder.join("shaders/demo.frag"), "uniform sampler2D g_Texture0;\nuniform sampler2D g_Texture7; // {\"combo\":\"MASK\"}\n#if MASK\nuniform float g_Time;\n#endif\nvoid main() { vec4 pixel = texSample2D(g_Texture0, vec2(0.5));\n#if MASK\npixel.a *= texSample2D(g_Texture7, vec2(0.5)).r * g_Time;\n#endif\ngl_FragColor = pixel; }").unwrap();
    let material = |textures: Value| {
        std::fs::write(
            folder.join("material.json"),
            serde_json::json!({"passes":[{"shader":"demo","textures":textures}]}).to_string(),
        )
        .unwrap()
    };
    let scene = Scene::parse(r#"{"objects":[{"image":"model.json"}]}"#).unwrap();
    let project = Project::open(&folder, None).unwrap();
    for unbound in [
        serde_json::json!(["same"]),
        serde_json::json!(["same", null]),
        serde_json::json!(["same", ""]),
    ] {
        material(unbound);
        let (layers, skipped) = project.layers_with_budget(&scene, 64);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert!(layers[0].secondary.is_empty());
        assert!(!layers[0].fragment.as_ref().unwrap().animated);
    }
    material(serde_json::json!([
        "same", null, null, null, null, null, null, "mask"
    ]));
    assert!(project.layers(&scene).0.is_empty());
    std::fs::write(
        folder.join("materials/mask.tex"),
        texture_format(4, 4, 9, 1),
    )
    .unwrap();
    let (layers, skipped) = project.layers_with_budget(&scene, 80);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(layers[0].secondary.len(), 1);
    assert_eq!(layers[0].secondary[0].slot, 7);
    assert!(layers[0].fragment.as_ref().unwrap().animated);
    assert!(project.layers_with_budget(&scene, 79).0.is_empty());
    material(serde_json::json!([null, "mask"]));
    assert!(project.layers(&scene).0.is_empty());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn default_resources_share_pixels_respect_overrides_and_observe_assets_and_budget() {
    let folder = folder("defaults");
    let assets = folder.join("assets");
    std::fs::create_dir_all(assets.join("materials")).unwrap();
    std::fs::create_dir_all(folder.join("materials")).unwrap();
    std::fs::create_dir_all(folder.join("shaders")).unwrap();
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    for model in ["a.json", "b.json"] {
        std::fs::write(folder.join(model), r#"{"material":"material.json"}"#).unwrap();
    }
    std::fs::write(folder.join("shaders/demo.frag"), "uniform sampler2D g_Texture0; // {\"default\":\"base\"}\nuniform sampler2D g_Texture7; // {\"default\":\"base\",\"combo\":\"MASK\"}\nvoid main() { gl_FragColor = texSample2D(g_Texture0, vec2(0.5)) * texSample2D(g_Texture7, vec2(0.5)); }").unwrap();
    std::fs::write(assets.join("materials/base.tex"), texture(4, 4)).unwrap();
    let project = Project::open(&folder, Some(&assets)).unwrap();
    let scene = Scene::parse(r#"{"objects":[{"image":"a.json"},{"image":"b.json"}]}"#).unwrap();
    for pass in [
        serde_json::json!({"shader":"demo"}),
        serde_json::json!({"shader":"demo","textures":[]}),
        serde_json::json!({"shader":"demo","textures":[null]}),
    ] {
        std::fs::write(
            folder.join("material.json"),
            serde_json::json!({"passes":[pass]}).to_string(),
        )
        .unwrap();
        let (layers, skipped) = project.layers_with_budget(&scene, 64);
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].secondary[0].slot, 7);
        assert!(Arc::ptr_eq(
            &layers[0].texture,
            &layers[0].secondary[0].texture
        ));
        assert!(Arc::ptr_eq(&layers[0].texture, &layers[1].texture));
        assert!(Arc::ptr_eq(
            layers[0].fragment.as_ref().unwrap(),
            layers[1].fragment.as_ref().unwrap()
        ));
        assert!(project.layers_with_budget(&scene, 63).0.is_empty());
    }
    std::fs::write(folder.join("materials/override.tex"), texture(2, 2)).unwrap();
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"shader":"demo","textures":["override"]}]}"#,
    )
    .unwrap();
    let (layers, skipped) = project.layers_with_budget(&scene, 80);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(layers[0].texture.width, 2);
    assert_eq!(layers[0].secondary[0].texture.width, 4);
    assert!(!Arc::ptr_eq(
        &layers[0].texture,
        &layers[0].secondary[0].texture
    ));
    assert!(project.layers_with_budget(&scene, 79).0.is_empty());
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"shader":"demo","textures":["missing"]}]}"#,
    )
    .unwrap();
    let (layers, skipped) = project.layers(&scene);
    assert!(layers.is_empty());
    assert!(skipped[0].contains("missing.tex"));
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"shader":"demo"}]}"#,
    )
    .unwrap();
    std::fs::remove_file(assets.join("materials/base.tex")).unwrap();
    assert!(project.layers(&scene).0.is_empty());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn malformed_passes_and_default_paths_fail_before_decoding_inputs() {
    let folder = folder("default-policy");
    std::fs::create_dir_all(folder.join("shaders")).unwrap();
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    let project = Project::open(&folder, None).unwrap();
    let scene = Scene::parse(r#"{"objects":[{"image":"model.json"}]}"#).unwrap();
    for (field, reason) in [
        (
            serde_json::json!({"stencil":true,"textures":["missing"]}),
            "pass field",
        ),
        (
            serde_json::json!({"blending":"unknown","textures":["missing"]}),
            "blending",
        ),
        (
            serde_json::json!({"blending":false,"textures":["missing"]}),
            "blending",
        ),
    ] {
        std::fs::write(
            folder.join("material.json"),
            serde_json::json!({"passes":[field]}).to_string(),
        )
        .unwrap();
        let (layers, skipped) = project.layers(&scene);
        assert!(layers.is_empty());
        assert!(skipped[0].contains(reason), "{skipped:?}");
    }
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"shader":"demo"}]}"#,
    )
    .unwrap();
    for name in [
        "../outside",
        "/etc/passwd",
        "bad\\path",
        "_rt_FullFrameBuffer",
    ] {
        let annotation = serde_json::json!({"default":name});
        std::fs::write(folder.join("shaders/demo.frag"), format!("uniform sampler2D g_Texture0; // {annotation}\nvoid main() {{ gl_FragColor = vec4(1); }}")).unwrap();
        let (layers, skipped) = project.layers(&scene);
        assert!(layers.is_empty());
        assert!(!skipped[0].contains("isn't in the project"), "{skipped:?}");
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn complete_material_results_and_errors_are_cached_only_for_one_preparation() {
    let folder = folder("material-results");
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    let file = folder.join("material.json");
    std::fs::write(
        &file,
        r#"{"passes":[{"textures":["base"],"blending":"normal"}]}"#,
    )
    .unwrap();
    let project = Project::open(&folder, None).unwrap();
    let mut resources = material::Resources::default();
    let first = project.material("material.json", &mut resources).unwrap();
    std::fs::remove_file(&file).unwrap();
    let cached = project.material("material.json", &mut resources).unwrap();
    assert_eq!(cached.names, first.names);
    assert_eq!(cached.blending, Blending::Opaque);
    let mut failed = material::Resources::default();
    let error = project
        .material("material.json", &mut failed)
        .err()
        .unwrap()
        .to_string();
    std::fs::write(
        &file,
        r#"{"passes":[{"textures":["replacement"],"blending":"additive"}]}"#,
    )
    .unwrap();
    assert_eq!(
        project
            .material("material.json", &mut failed)
            .err()
            .unwrap()
            .to_string(),
        error
    );
    let reloaded = project
        .material("material.json", &mut material::Resources::default())
        .unwrap();
    assert_eq!(reloaded.blending, Blending::Additive);
    assert_eq!(reloaded.names, [Some("replacement".into())]);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn a_shared_invalid_texture_is_not_read_or_decoded_again_for_other_models() {
    let folder = folder("texture-results");
    std::fs::create_dir(folder.join("materials")).unwrap();
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    let file = folder.join("materials/shared.tex");
    std::fs::write(&file, b"broken").unwrap();
    let project = Project::open(&folder, None).unwrap();
    let mut resources = material::Resources::default();
    let mut remaining = 64;
    let error = project
        .texture("shared", &mut resources, &mut remaining, true)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("materials/shared.tex"));
    assert_eq!(remaining, 64);
    std::fs::write(&file, texture(4, 4)).unwrap();
    assert_eq!(
        project
            .texture("shared", &mut resources, &mut remaining, true)
            .err()
            .unwrap()
            .to_string(),
        error
    );
    assert_eq!(remaining, 64);
    assert!(
        project
            .texture(
                "shared",
                &mut material::Resources::default(),
                &mut remaining,
                true
            )
            .is_ok()
    );
    assert_eq!(remaining, 0);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn cached_failure_text_stays_bounded_before_it_is_copied_to_other_models() {
    let folder = folder("cached-errors");
    std::fs::create_dir(folder.join("shaders")).unwrap();
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    for name in ["a.json", "b.json"] {
        std::fs::write(folder.join(name), r#"{"material":"material.json"}"#).unwrap();
    }
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"shader":"demo","textures":["base"]}]}"#,
    )
    .unwrap();
    std::fs::write(
        folder.join("shaders/demo.frag"),
        format!(
            "void main() {{ float apolline_{} = 1.0; gl_FragColor = vec4(1); }}",
            "x".repeat(10000)
        ),
    )
    .unwrap();
    let project = Project::open(&folder, None).unwrap();
    let scene = Scene::parse(r#"{"objects":[{"image":"a.json"},{"image":"b.json"}]}"#).unwrap();
    let mut models = HashMap::new();
    let mut resources = material::Resources::default();
    let mut remaining = 64;
    for image in scene.images() {
        assert!(
            project
                .layer(image, &mut models, &mut resources, &mut remaining, true)
                .is_err()
        );
    }
    assert!(
        models
            .values()
            .all(|value| value.as_ref().err().unwrap().chars().count() <= 1800)
    );
    assert!(
        project
            .material("material.json", &mut resources)
            .err()
            .unwrap()
            .to_string()
            .chars()
            .count()
            <= 1800
    );
    assert_eq!(remaining, 64);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn a_stock_image_pass_with_render_state_draws_as_a_plain_layer() {
    let folder = folder("stock-pass");
    std::fs::create_dir_all(folder.join("materials")).unwrap();
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    std::fs::write(folder.join("model.json"), r#"{"material":"material.json"}"#).unwrap();
    std::fs::write(
        folder.join("material.json"),
        r#"{"passes":[{"shader":"genericimage2","textures":["sky"],"blending":"translucent","cullmode":"nocull","depthtest":"disabled","depthwrite":"disabled"}]}"#,
    )
    .unwrap();
    std::fs::write(folder.join("materials/sky.tex"), texture(2, 2)).unwrap();
    let project = Project::open(&folder, None).unwrap();
    let scene = Scene::parse(r#"{"objects":[{"image":"model.json"}]}"#).unwrap();
    let (layers, skipped) = project.layers(&scene);
    assert!(skipped.is_empty(), "{skipped:?}");
    assert_eq!(layers.len(), 1);
    assert!(layers[0].fragment.is_none());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn a_flat_layer_is_its_colour_at_its_size() {
    let folder = folder("flat");
    std::fs::write(folder.join("project.json"), "{}").unwrap();
    std::fs::write(folder.join("scene.json"), r#"{"general":{"orthogonalprojection":{"width":100,"height":100}},"objects":[{"image":"solid.json","size":"40 20","color":"1 0 0"}]}"#).unwrap();
    std::fs::write(
        folder.join("solid.json"),
        r#"{"material":"flat.json","solidlayer":true}"#,
    )
    .unwrap();
    std::fs::write(
        folder.join("flat.json"),
        r#"{"passes":[{"shader":"flat","blending":"translucent","cullmode":"nocull"}]}"#,
    )
    .unwrap();
    let prepared = Project::open(&folder, None).unwrap().prepare().unwrap();
    let layer = &prepared.layers[0];
    assert_eq!(
        (layer.texture.image_width, layer.texture.image_height),
        (40, 20)
    );
    assert_eq!(layer.texture.images[0][0].pixels, [255; 4]);
    assert_eq!(layer.image.color, [1.0, 0.0, 0.0]);
    std::fs::remove_dir_all(folder).unwrap();
}
