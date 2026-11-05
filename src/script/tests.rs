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
