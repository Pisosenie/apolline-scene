use super::{folder, texture};
use crate::Project;
use std::path::Path;

/// A scene with one layer carrying `effects`, a pass-through effect `copy` and a tint effect
/// whose material takes a colour and a second texture.
fn scene(name: &str, effects: &str) -> std::path::PathBuf {
    let folder = folder(name);
    for directory in [
        "materials/effects",
        "materials/util",
        "shaders/effects",
        "effects/copy",
        "effects/tint",
    ] {
        std::fs::create_dir_all(folder.join(directory)).unwrap();
    }
    let write = |path: &str, text: &str| std::fs::write(folder.join(path), text).unwrap();
    write(
        "project.json",
        r#"{"general":{"properties":{"shown":{"type":"bool","value":true}}}}"#,
    );
    write(
        "scene.json",
        &format!(
            r#"{{"general":{{"orthogonalprojection":{{"width":10,"height":10}}}},"objects":[{{"image":"model.json","effects":{effects}}}]}}"#
        ),
    );
    write("model.json", r#"{"material":"material.json"}"#);
    write("material.json", r#"{"passes":[{"textures":["picture"]}]}"#);
    write(
        "effects/copy/effect.json",
        r#"{"name":"copy","group":"x","passes":[{"material":"materials/effects/copy.json"}]}"#,
    );
    write(
        "materials/effects/copy.json",
        r#"{"passes":[{"shader":"effects/copy","blending":"normal","textures":[null]}]}"#,
    );
    write(
        "shaders/effects/copy.frag",
        "varying vec2 v_TexCoord;\nuniform sampler2D g_Texture0;\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord); }",
    );
    write(
        "effects/tint/effect.json",
        r#"{"passes":[{"material":"materials/effects/tint.json"}]}"#,
    );
    write(
        "materials/effects/tint.json",
        r#"{"passes":[{"shader":"effects/tint","textures":[null,"util/white"],"combos":{"STRONG":0},"constantshadervalues":{"color":"0 0 0"}}]}"#,
    );
    write(
        "shaders/effects/tint.frag",
        "// [COMBO] {\"combo\":\"STRONG\",\"default\":0}\nvarying vec2 v_TexCoord;\nuniform sampler2D g_Texture0;\nuniform sampler2D g_Texture1;\nuniform vec3 u_Color; // {\"material\":\"color\",\"default\":\"1 1 1\"}\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord) * texSample2D(g_Texture1, v_TexCoord) * vec4(u_Color * float(STRONG + 1), 1.0); }",
    );
    std::fs::write(folder.join("materials/picture.tex"), texture(1, 1)).unwrap();
    std::fs::write(folder.join("materials/util/white.tex"), texture(1, 1)).unwrap();
    folder
}

fn refusal(folder: &Path) -> String {
    match Project::open(folder, None).unwrap().prepare() {
        Ok(_) => "drawn".into(),
        Err(error) => error.to_string(),
    }
}

#[test]
fn effects_become_passes_with_the_instance_overriding_its_material() {
    let folder = scene(
        "effects",
        r#"[{"file":"effects/copy/effect.json"},
            {"file":"effects/tint/effect.json","visible":{"user":"shown","value":true},
             "passes":[{"combos":{"STRONG":1},"constantshadervalues":{"color":"1 0.5 0"}}]},
            {"file":"effects/tint/effect.json","visible":false}]"#,
    );
    let mut project = Project::open(&folder, None).unwrap();
    let prepared = project.prepare().unwrap();
    let passes = &prepared.layers[0].effects;
    assert_eq!(passes.len(), 2);
    assert!(passes[0].inputs.is_empty());
    assert_eq!(passes[1].inputs[0].slot, 1);
    assert!(passes[1].fragment.source.contains("#define STRONG 1"));
    assert_eq!(
        passes[1].fragment.constants[0].1.floats()[..3],
        [1.0, 0.5, 0.0]
    );

    project
        .set_properties(&crate::properties::Values::from([(
            "shown".into(),
            serde_json::json!(false),
        )]))
        .unwrap();
    assert_eq!(project.prepare().unwrap().layers[0].effects.len(), 1);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn what_effects_do_that_isnt_drawn_is_refused_by_name() {
    let cases = [
        (
            r#"[{"file":"effects/copy/effect.json","mystery":1}]"#,
            "",
            "unsupported field mystery",
        ),
        (
            r#"[{"file":"effects/copy/effect.json","passes":[{},{}]}]"#,
            "",
            "passes don't match",
        ),
        (
            r#"[{"file":"effects/copy/effect.json","passes":[{"textures":["picture"]}]}]"#,
            "",
            "over the layer's picture",
        ),
        (
            r#"[{"file":"effects/tint/effect.json","passes":[{"constantshadervalues":{"color":{"value":"1 1 1","animation":{}}}}]}]"#,
            "",
            "animated effect values",
        ),
        (
            r#"[{"file":"effects/copy/effect.json"}]"#,
            "fbos",
            "effect copy: unsupported field fbos",
        ),
        (
            r#"[{"file":"effects/copy/effect.json"}]"#,
            "target",
            "unsupported field target",
        ),
        (
            r#"[{"file":"effects/copy/effect.json"}]"#,
            "blend",
            "passes that blend",
        ),
        (
            r#"[{"file":"effects/missing/effect.json"}]"#,
            "",
            "effect missing",
        ),
    ];
    for (at, (effects, change, expected)) in cases.into_iter().enumerate() {
        let folder = scene(&format!("effect-refusal-{at}"), effects);
        let write = |path: &str, text: &str| std::fs::write(folder.join(path), text).unwrap();
        match change {
            "fbos" => write(
                "effects/copy/effect.json",
                r#"{"passes":[{"material":"materials/effects/copy.json"}],"fbos":[{"name":"half"}]}"#,
            ),
            "target" => write(
                "effects/copy/effect.json",
                r#"{"passes":[{"material":"materials/effects/copy.json","target":"half"}]}"#,
            ),
            "blend" => write(
                "materials/effects/copy.json",
                r#"{"passes":[{"shader":"effects/copy","blending":"additive"}]}"#,
            ),
            _ => {}
        }
        let refused = refusal(&folder);
        assert!(refused.contains(expected), "{expected}: {refused}");
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[test]
fn a_pointer_reader_waits_for_a_host_that_gives_the_pointer() {
    let folder = scene("effect-pointer", r#"[{"file":"effects/copy/effect.json"}]"#);
    std::fs::write(
        folder.join("shaders/effects/copy.frag"),
        "varying vec2 v_TexCoord;\nuniform sampler2D g_Texture0;\nuniform vec2 g_PointerPosition;\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord + g_PointerPosition); }",
    )
    .unwrap();
    let project = Project::open(&folder, None).unwrap();
    assert!(refusal(&folder).contains("pointer parallax"));
    let snapshot = project.prepare_snapshot().unwrap();
    assert!(snapshot.layers[0].effects[0].fragment.reads.pointer);
    std::fs::remove_dir_all(folder).unwrap();
}
