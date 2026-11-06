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

fn after(delta: f32) -> Moment<'static> {
    Moment { delta, ..at(0.0) }
}

/// Keys at the given frames and values, with handles `ease` (flat) or straight.
fn keys(points: &[(f32, f32)], ease: bool) -> String {
    let handle = |x: i32| format!(r#"{{"enabled": {ease}, "x": {x}, "y": 0}}"#);
    let keys: Vec<String> = points
        .iter()
        .map(|(frame, value)| {
            format!(
                r#"{{"frame": {frame}, "value": {value}, "back": {}, "front": {}}}"#,
                handle(-1),
                handle(1)
            )
        })
        .collect();
    format!("[{}]", keys.join(","))
}

#[test]
fn timelines_play_on_objects_values_each_frame() {
    let line = keys(&[(0.0, 0.0), (30.0, 100.0)], false);
    let ease = keys(&[(0.0, 0.0), (30.0, 100.0)], true);
    let turn = keys(&[(0.0, 0.0), (30.0, std::f32::consts::PI)], false);
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "origin": {{"value": "10 20 0", "animation":
              {{"c0": {line}, "options": {{"fps": 30, "length": 60, "mode": "loop"}}, "relative": true}}}},
              "alpha": {{"value": 1, "animation": {{"c0": {ease}, "options": {{"fps": 30, "length": 30, "mode": "single"}}}}}},
              "angles": {{"value": "0 0 0", "animation": {{"c2": {turn}, "options": {{"fps": 30, "length": 30, "mode": "mirror"}}}}}}}}]"#
    ));
    assert!(scene.missing().is_empty(), "{:?}", scene.missing());
    let mut stage = stage(&scene);
    // A quarter second in: frame 7.5 of 30.
    let pose = stage.tick(after(0.25))[0];
    assert!(
        (pose.origin[0] - (10.0 + 25.0)).abs() < 1e-3,
        "{:?}",
        pose.origin
    );
    assert_eq!(pose.origin[1], 20.0);
    // Flat handles ease: a quarter of the way in time is 15.625% of the way.
    assert!((pose.alpha - 15.625).abs() < 1e-3, "{}", pose.alpha);
    assert!((pose.angle - std::f32::consts::FRAC_PI_4).abs() < 1e-4);
    // A second and a quarter in: the loop past its last key, holding it, the single one held
    // at its end, and the mirror on its way back.
    let pose = stage.tick(after(1.0))[0];
    assert_eq!(pose.origin[0], 110.0);
    assert_eq!(pose.alpha, 100.0);
    assert!(
        (pose.angle - std::f32::consts::FRAC_PI_4 * 3.0).abs() < 1e-4,
        "{}",
        pose.angle
    );
}

#[test]
fn a_loop_that_wraps_runs_from_its_last_key_back_to_its_first() {
    let line = keys(&[(10.0, 0.0), (30.0, 100.0)], false);
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "alpha": {{"value": 1, "animation": {{"c0": {line},
            "options": {{"fps": 10, "length": 40, "mode": "loop", "wraploop": true}}}}}}}}]"#
    ));
    let mut stage = stage(&scene);
    // Frame 35, a quarter of the way from the last key at 30 to the first again at 50.
    assert!((stage.tick(after(3.5))[0].alpha - 75.0).abs() < 1e-3);
    // Frame 5 of the next time round, still on the way down to it.
    assert!((stage.tick(after(1.0))[0].alpha - 25.0).abs() < 1e-3);
}

#[test]
fn scripts_see_animated_values_and_start_paused_timelines() {
    let line = keys(&[(0.0, 0.0), (10.0, 1.0)], false);
    let start = scripted(
        "export function init() { thisObject.getAnimation('later').play(); }
         export function update(value) { return value * 2; }",
    );
    let scene = scene(&format!(
        r#"[{{"image": "a.json", "effects": [{{"file": "a", "passes": [{{"constantshadervalues": {{
            "speed": {{"value": 0, "script": {start}, "animation": {{"c0": {line},
                "options": {{"fps": 10, "length": 10, "mode": "single", "startpaused": true, "name": "later"}}}}}},
            "still": {{"value": 0, "animation": {{"c0": {line},
                "options": {{"fps": 10, "length": 10, "mode": "single", "startpaused": true}}}}}}}}}}]}}]}}]"#
    ));
    assert!(scene.missing().is_empty(), "{:?}", scene.missing());
    assert_eq!(scene.driven.len(), 2);
    let mut stage = stage(&scene);
    stage.tick(after(0.5));
    let values = stage.effects().1;
    // Played by its script and doubled by it; the other waits at its start.
    assert!((values[0][0] - 1.0).abs() < 1e-4, "{:?}", values[0]);
    assert_eq!(values[1][0], 0.0);
}

#[test]
fn a_script_on_the_bloom_starts_from_its_strength_and_has_no_layer() {
    let source = scripted(
        "let first; export function init(value) { first = value; }
         export function update() { return thisLayer === null ? first * engine.runtime : -1; }",
    );
    let scene = Scene::parse(&format!(
        r#"{{"general": {{"orthogonalprojection": {{"width": 400, "height": 200}}, "bloom": true,
            "bloomstrength": {{"script": {source}, "value": 1.5}}}},
            "objects": [{{"image": "a.json"}}]}}"#
    ))
    .unwrap();
    assert!(scene.missing().is_empty());
    assert_eq!(scene.scripts[0].label(), "general.bloomstrength");
    let mut stage = stage(&scene);
    stage.tick(at(2.0));
    assert_eq!(stage.effects().1[0][0], 3.0);
}
