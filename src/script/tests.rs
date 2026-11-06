use super::*;

fn scene(objects: &str) -> Scene {
    Scene::parse(&format!(
        r#"{{"general": {{"orthogonalprojection": {{"width": 400, "height": 200}}}}, "objects": {objects}}}"#
    ))
    .unwrap()
}

fn scripted(source: &str) -> String {
    serde_json::to_string(source).unwrap()
}

fn at(time: f64) -> Moment<'static> {
    Moment {
        time,
        delta: 1.0 / 60.0,
        daytime: 0.25,
        screen: (800, 400),
        spectrum: None,
    }
}

fn stage(scene: &Scene) -> Stage {
    Stage::new(scene, &serde_json::Map::new()).unwrap().unwrap()
}

#[test]
fn a_scene_without_scripts_has_no_stage() {
    let scene = scene(r#"[{"image": "a.json"}]"#);
    assert!(
        Stage::new(&scene, &serde_json::Map::new())
            .unwrap()
            .is_none()
    );
}

#[test]
fn update_gets_the_value_and_returns_the_new_one_each_frame() {
    let source = scripted(
        "export function update(value) { value.x = 10 * engine.runtime + 1; return value; }",
    );
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "origin": {{"script": {source}, "value": "5 6 0"}}}}]"#
    ));
    let mut stage = stage(&scene);
    assert_eq!(stage.tick(at(2.0))[0].origin, [21.0, 6.0, 0.0]);
    assert_eq!(stage.tick(at(3.0))[0].origin, [31.0, 6.0, 0.0]);
}

#[test]
fn a_turning_parent_carries_its_children_in_degrees() {
    let source =
        scripted("export function update(value) { value.z = 90 * engine.runtime; return value; }");
    let scene = scene(&format!(
        r#"[{{"id": 1, "origin": "100 100 0", "angles": {{"script": {source}, "value": "0 0 0"}}}},
            {{"id": 2, "parent": 1, "image": "a.json", "origin": "10 0 0"}}]"#
    ));
    let mut stage = stage(&scene);
    let child = stage.tick(at(1.0))[1];
    assert!((child.origin[0] - 100.0).abs() < 1e-3 && (child.origin[1] - 110.0).abs() < 1e-3);
    assert!((child.angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
}

#[test]
fn script_properties_imports_and_other_layers_are_reachable() {
    let source = scripted(
        "import * as WEMath from 'WEMath';
         export var scriptProperties = createScriptProperties()
             .addSlider({ name: 'speed', value: 1 })
             .addCheckbox({ name: 'hide', value: false })
             .finish();
         export function update(value) {
             const other = thisScene.getLayer('other');
             other.visible = !scriptProperties.hide;
             other.alpha = WEMath.mix(0, 1, 0.25);
             return WEMath.clamp(scriptProperties.speed, 0, 3);
         }",
    );
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "alpha": {{"script": {source}, "scriptproperties": {{"speed": {{"user": "s", "value": 7}}, "hide": true}}, "value": 1}}}},
            {{"name": "other", "image": "b.json"}}]"#
    ));
    let mut stage = stage(&scene);
    let poses = stage.tick(at(0.5));
    assert_eq!(poses[0].alpha, 3.0);
    assert!(!poses[1].visible);
    assert_eq!(poses[1].alpha, 0.25);
}

#[test]
fn timers_and_the_audio_spectrum_reach_scripts() {
    let source = scripted(
        "let audio = engine.registerAudioBuffers(engine.AUDIO_RESOLUTION_16);
         let fired = false;
         engine.setTimeout(() => { fired = true; }, 500);
         export function update(value) {
             return new Vec3(audio.left[0], audio.right[15], fired ? 1 : 0);
         }",
    );
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "scale": {{"script": {source}, "value": "1 1 1"}}}}]"#
    ));
    let mut stage = stage(&scene);
    assert_eq!(stage.needs(), (false, true));
    let mut spectrum = [[0.0; 64]; 2];
    spectrum[0][0] = 0.5;
    spectrum[0][1] = 0.25;
    spectrum[1][63] = 1.0;
    let early = Moment {
        spectrum: Some(&spectrum),
        ..at(0.25)
    };
    assert_eq!(stage.tick(early)[0].scale, [0.1875, 0.25, 0.0]);
    assert_eq!(stage.tick(at(0.6))[0].scale[2], 1.0);
}

#[test]
fn a_script_that_asks_for_what_isnt_here_names_its_layer() {
    let source = scripted(
        "export function update(value) { thisLayer.getTextureAnimation().play(); return value; }",
    );
    let clock = scene(&format!(
        r#"[{{"name": "clock hand", "image": "a.json", "visible": {{"script": {source}, "value": true}}}}]"#
    ));
    let error = Stage::new(&clock, &serde_json::Map::new())
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("clock hand's visible"), "{error}");
    assert!(error.contains("getTextureAnimation"), "{error}");
    let broken = scripted("export function update(value) { return value +; }");
    let broken = scene(&format!(
        r#"[{{"image": "a.json", "alpha": {{"script": {broken}, "value": 1}}}}]"#
    ));
    assert!(Stage::new(&broken, &serde_json::Map::new()).is_err());
}

#[test]
fn a_script_that_never_returns_is_stopped() {
    let source = scripted(
        "export function update(value) { if (engine.runtime > 1) { for (;;) {} } return value; }",
    );
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "alpha": {{"script": {source}, "value": 0.5}}}}]"#
    ));
    let mut stage = stage(&scene);
    let begun = std::time::Instant::now();
    assert_eq!(stage.tick(at(2.0))[0].alpha, 0.5);
    assert!(begun.elapsed() < std::time::Duration::from_secs(1));
    // It doesn't run again, so later frames cost nothing.
    let begun = std::time::Instant::now();
    stage.tick(at(3.0));
    assert!(begun.elapsed() < std::time::Duration::from_millis(20));
}

#[test]
fn a_frame_cut_short_keeps_other_scripts_timers_and_the_world() {
    let ticking = scripted(
        "let count = 0;
         engine.setInterval(() => { count += 1; }, 100);
         export function update(value) { return new Vec3(count, 0, 0); }",
    );
    let stuck = scripted(
        "engine.setTimeout(() => { for (;;) {} }, 1000);
         export function update(value) {
             try { globalThis.__world = null; } catch (error) {}
             return value;
         }",
    );
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "origin": {{"script": {ticking}, "value": "0 0 0"}}}},
            {{"image": "b.json", "alpha": {{"script": {stuck}, "value": 1}}}}]"#
    ));
    let mut stage = stage(&scene);
    stage.tick(at(0.5));
    let counted = stage.tick(at(0.95))[0].origin[0];
    assert!(counted > 0.0);
    // The stuck timer is cut short this frame, and stops; the interval goes on after it.
    stage.tick(at(1.05));
    assert!(stage.tick(at(1.5))[0].origin[0] > counted);
}

#[test]
fn a_script_may_start_listening_late() {
    let source = scripted(
        "let audio = null;
         export function update(value) {
             if (engine.runtime > 1) { audio = audio || engine.registerAudioBuffers(16); }
             return value;
         }",
    );
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "alpha": {{"script": {source}, "value": 1}}}}]"#
    ));
    let mut stage = stage(&scene);
    assert_eq!(stage.needs(), (false, false));
    stage.tick(at(2.0));
    assert_eq!(stage.needs(), (false, true));
}

#[test]
fn promises_that_never_settle_are_cut_short_too() {
    let source = scripted(
        "export function update(value) {
             if (engine.runtime > 1) { Promise.resolve().then(() => { for (;;) {} }); }
             return value;
         }",
    );
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "alpha": {{"script": {source}, "value": 1}}}}]"#
    ));
    let mut stage = stage(&scene);
    let begun = std::time::Instant::now();
    stage.tick(at(2.0));
    assert!(begun.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn text_layers_words_change_through_their_scripts_and_others() {
    let clock = scripted(
        "let ticks = 0; export function update(value) { ticks += 1; return `${value.length}:${ticks}`; }",
    );
    let caption = scripted(
        "export function update(value) { \
         thisScene.getLayer('label').text = thisScene.getLayer('clock').text + '!'; \
         thisScene.getLayer('picture').text = 'ignored'; \
         return value; }",
    );
    let scene = scene(&format!(
        r#"[{{"name": "clock", "text": {{"script": {clock}, "value": "ab"}}, "font": "a.ttf"}},
            {{"name": "label", "text": "x", "font": "a.ttf"}},
            {{"name": "picture", "image": "a.json", "origin": {{"script": {caption}, "value": "0 0 0"}}}}]"#
    ));
    let mut stage = stage(&scene);
    assert_eq!(
        stage.written(),
        [(0, "2:1".to_owned()), (1, "2:1!".to_owned())]
    );
    stage.tick(at(0.5));
    assert_eq!(
        stage.written(),
        [(0, "3:2".to_owned()), (1, "3:2!".to_owned())]
    );
}

#[test]
fn words_cut_inside_a_character_come_out_whole() {
    let typing = scripted(
        "export function update() { return '\\uD83D\\uDE00ok'.slice(1) + 'x'.repeat(40000); }",
    );
    let scene = scene(&format!(
        r#"[{{"name": "typing", "text": {{"script": {typing}, "value": ""}}, "font": "a.ttf"}}]"#
    ));
    let mut stage = stage(&scene);
    let written = stage.written();
    assert_eq!(written.len(), 1);
    assert!(
        written[0].1.starts_with("\u{FFFD}ok"),
        "{:?}",
        &written[0].1[..8]
    );
    assert!(written[0].1.chars().count() <= crate::text::CHARACTERS);
}

#[test]
fn effects_are_shown_hidden_and_given_values_by_their_scripts() {
    let show = scripted("export function update() { return engine.runtime >= 1; }");
    let hide = scripted("export function update() { thisObject.visible = false; }");
    let tint = scripted(
        "let first;
         export function init(value) { first = value; thisLayer.getEffect(3).visible = true; }
         export function update(value) { return first.multiply(1 + engine.runtime); }",
    );
    let speed = scripted("export function update(value) { return value + 1; }");
    let flag = scripted("export function update(value) { return !value; }");
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "effects": [
            {{"file": "a", "visible": {{"script": {show}, "value": false}}}},
            {{"file": "b", "visible": {{"script": {hide}, "value": true}},
              "passes": [{{"constantshadervalues": {{"color": {{"script": {tint}, "value": "0.1 0.2 0.3"}},
                                                     "speed": {{"script": {speed}, "value": 2}}}}}},
                         {{"constantshadervalues": {{"on": {{"script": {flag}, "value": true}}}}}}]}},
            {{"file": "c", "visible": false}},
            {{"file": "d", "visible": false}}
        ]}}]"#
    ));
    // A script asks for effects by place, so the hidden ones stay for it to show.
    assert_eq!(scene.effects().len(), 4);
    let mut stage = stage(&scene);
    stage.tick(at(1.0));
    let (shown, values) = stage.effects();
    assert_eq!(shown, [true, false, false, true]);
    assert_eq!(values.len(), 3);
    let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
    assert!(close(values[0], [0.2, 0.4, 0.6, 0.0]), "{:?}", values[0]);
    // Two frames have run: the first when the stage was made.
    assert_eq!(values[1][0], 4.0);
    assert_eq!(values[2][0], 1.0);
    stage.tick(at(2.0));
    assert_eq!(stage.effects().1[2][0], 0.0);
}

#[test]
fn hidden_effects_nothing_can_show_are_left_out() {
    let speed = scripted("export function update(value) { return value; }");
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "effects": [
            {{"file": "a", "visible": false,
              "passes": [{{"constantshadervalues": {{"speed": {{"script": {speed}, "value": 2}}}}}}]}},
            {{"file": "b"}}
        ]}}]"#
    ));
    let effects = scene.effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].1.index, 1);
    assert!(scene.scripts.is_empty());
}

#[test]
fn an_effect_script_that_fails_is_named_by_where_it_is() {
    let broken = scripted("export function update() { return thisObject.getMaterial(0); }");
    let scene = scene(&format!(
        r#"[{{"name": "logo", "image": "a.json", "effects": [{{"file": "a",
            "passes": [{{"constantshadervalues": {{"speed": {{"script": {broken}, "value": 2}}}}}}]}}]}}]"#
    ));
    let error = Stage::new(&scene, &serde_json::Map::new())
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("logo's effects[0].passes[0].speed") && error.contains("effect.getMaterial"),
        "{error}"
    );
}

#[test]
fn values_keep_their_shape_until_scripts_change_them() {
    let same = scripted("export function update(value) { return value; }");
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "effects": [{{"file": "a", "passes": [{{"constantshadervalues": {{
            "colour": {{"script": {same}, "value": [0.1, 0.2, 0.3]}},
            "on": {{"script": {same}, "value": "true"}},
            "point": {{"script": {same}, "value": "1 2"}}}}}}]}}]}}]"#
    ));
    let mut stage = stage(&scene);
    stage.tick(at(1.0));
    let values = stage.effects().1;
    assert_eq!(values[0][..3], [0.1, 0.2, 0.3]);
    assert_eq!(values[1][0], 1.0);
    assert_eq!(values[2][..2], [1.0, 2.0]);
}
