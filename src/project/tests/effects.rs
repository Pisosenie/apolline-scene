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
fn scripted_effects_are_kept_and_know_what_their_values_feed() {
    let folder = scene(
        "effects-scripted",
        r#"[{"file":"effects/copy/effect.json","visible":{"script":"export function update() { return true; }","value":false}},
            {"file":"effects/copy/effect.json","visible":false},
            {"file":"effects/tint/effect.json",
             "passes":[{"constantshadervalues":{"color":{"script":"export function update(v) { return v; }","value":"1 0.5 0"}}}]}]"#,
    );
    let project = Project::open(&folder, None).unwrap();
    let prepared = project.prepare().unwrap();
    let passes = &prepared.layers[0].effects;
    let placed: Vec<(usize, usize, bool)> = passes
        .iter()
        .map(|pass| (pass.effect, pass.pass, pass.visible))
        .collect();
    assert_eq!(placed, [(0, 0, false), (2, 0, true)]);
    let tint = &passes[1].fragment;
    assert_eq!(tint.keys, [Some("color".to_owned())]);
    assert_eq!(tint.constants[0].1.floats()[..3], [1.0, 0.5, 0.0]);
    assert_eq!(prepared.scene.scripts.len(), 2);
    let stage = prepared.stage().unwrap().unwrap();
    assert_eq!(stage.effects().0, [true, true]);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn a_hidden_effect_that_cant_be_drawn_fails_only_the_scripts_that_show_it() {
    let effects = |script: &str| {
        format!(
            r#"[{{"file":"effects/copy/effect.json","visible":{{"script":{},"value":false}}}},
                {{"file":"effects/tint/effect.json"}}]"#,
            serde_json::to_string(script).unwrap()
        )
    };
    let only_on_media =
        "export function mediaThumbnailChanged(event) { thisObject.visible = event.hasThumbnail; }";
    for (at, (script, shown)) in [
        (only_on_media, false),
        ("export function update() { return true; }", true),
    ]
    .into_iter()
    .enumerate()
    {
        let folder = scene(&format!("effects-undrawn-{at}"), &effects(script));
        std::fs::write(
            folder.join("effects/copy/effect.json"),
            r#"{"passes":[{"material":"materials/effects/copy.json"}],"fbos":[{"name":"half"}]}"#,
        )
        .unwrap();
        let project = Project::open(&folder, None).unwrap();
        if shown {
            let refused = refusal(&folder);
            assert!(
                refused.contains("effects[0].visible")
                    && refused.contains("can't be drawn: effect copy: unsupported field fbos"),
                "{refused}"
            );
        } else {
            let prepared = project.prepare().unwrap();
            let layer = &prepared.layers[0];
            assert_eq!(layer.effects.len(), 1);
            assert_eq!(layer.effects[0].effect, 1);
            assert_eq!(layer.undrawn.len(), 1);
            assert!(prepared.stage().unwrap().is_some());
        }
        std::fs::remove_dir_all(folder).unwrap();
    }
}

#[test]
fn hidden_effects_get_what_shown_ones_leave() {
    let show = r#"{"script":"export function update() { return false; }","value":false}"#;
    let shown: Vec<String> = (0..32)
        .map(|_| r#"{"file":"effects/copy/effect.json"}"#.to_owned())
        .collect();
    let folder = scene(
        "effects-hidden-leftovers",
        &format!(
            r#"[{{"file":"effects/tint/effect.json","visible":{show}}},{}]"#,
            shown.join(",")
        ),
    );
    let prepared = Project::open(&folder, None).unwrap().prepare().unwrap();
    let layer = &prepared.layers[0];
    assert_eq!(layer.effects.len(), 32);
    assert_eq!(layer.effects[0].effect, 1);
    assert_eq!(layer.undrawn.len(), 1);
    assert!(layer.undrawn[0].1.contains("more than 32"));
    std::fs::remove_dir_all(&folder).unwrap();

    // One that reads the pointer, on a host that doesn't give it.
    let folder = scene(
        "effects-hidden-pointer",
        &format!(r#"[{{"file":"effects/copy/effect.json","visible":{show}}}]"#),
    );
    std::fs::write(
        folder.join("shaders/effects/copy.frag"),
        "varying vec2 v_TexCoord;\nuniform sampler2D g_Texture0;\nuniform vec2 g_PointerPosition;\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord + g_PointerPosition); }",
    )
    .unwrap();
    let prepared = Project::open(&folder, None).unwrap().prepare().unwrap();
    assert!(prepared.layers[0].effects.is_empty());
    assert!(prepared.layers[0].undrawn[0].1.contains("pointer"));
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
