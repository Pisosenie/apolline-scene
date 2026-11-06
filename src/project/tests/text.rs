use super::folder;
use crate::Project;
use crate::project::Blending;

const FONT: &[u8] = include_bytes!("../../../tests/fonts/Cantarell-Regular.ttf");

fn project(name: &str, scene: &str) -> std::path::PathBuf {
    let folder = folder(name);
    std::fs::create_dir_all(folder.join("fonts")).unwrap();
    let write = |path: &str, text: &str| std::fs::write(folder.join(path), text).unwrap();
    write(
        "project.json",
        r#"{"general":{"properties":{"size":{"type":"slider","value":24,"min":1,"max":100}}}}"#,
    );
    write("scene.json", scene);
    std::fs::write(folder.join("fonts/words.ttf"), FONT).unwrap();
    folder
}

#[test]
fn text_layers_draw_their_words_in_the_projects_font() {
    let folder = project(
        "text",
        r#"{"general":{"orthogonalprojection":{"width":800,"height":600}},"objects":[
            {"name":"title","text":"Hello","font":"fonts/words.ttf","origin":"100 300 0",
             "pointsize":{"user":"size","value":12},"padding":16,"horizontalalign":"left"},
            {"name":"clock","font":"fonts/words.ttf","pointsize":12,"padding":"0 0","visible":false,
             "text":{"value":"--:--","script":"export function update() { thisLayer.visible = true; return '12:34'; }"}}
        ]}"#,
    );
    let project = Project::open(&folder, None).unwrap();
    let prepared = project.prepare().unwrap();
    assert_eq!(prepared.layers.len(), 2);
    let title = &prepared.layers[0];
    assert!(title.text.is_some() && title.blending == Blending::Translucent);
    let (width, height) = title.image.size.unwrap();
    // 24 points from the project's property are 100 scene pixels to the em.
    assert!(height > 100.0 + 32.0 && height < 160.0 + 32.0, "{height}");
    assert!(width > 2.0 * 100.0, "{width}");
    assert!((title.image.anchor[0] - (-0.5 + 16.0 / width)).abs() < 1e-5);
    assert_eq!(title.image.anchor[1], 0.0);
    // A hidden text layer with a script is kept for it to show, and its words follow it.
    assert_eq!(prepared.layers[1].object, 1);
    let mut stage = prepared.stage().unwrap().unwrap();
    assert_eq!(stage.written(), [(1, "12:34".to_owned())]);
    assert!(stage.written().is_empty(), "words are written once");
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn system_fonts_stand_in_with_wallpaper_engines_own_and_missing_fonts_refuse() {
    let scene = r#"{"general":{"orthogonalprojection":{"width":800,"height":600}},"objects":[
        {"name":"label","text":"Hi","font":"systemfont_nonesuch"}]}"#;
    let folder = project("system-font", scene);
    let assets = folder.join("assets");
    std::fs::create_dir_all(assets.join("fonts")).unwrap();
    std::fs::write(assets.join("fonts/NotoSans-Regular.ttf"), FONT).unwrap();
    let project = Project::open(&folder, Some(&assets)).unwrap();
    assert!(project.prepare().unwrap().layers[0].text.is_some());
    let bare = Project::open(&folder, None).unwrap();
    let refused = bare.prepare().err().unwrap().to_string();
    assert!(
        refused.contains("label") && refused.contains("nonesuch"),
        "{refused}"
    );
    std::fs::write(
        folder.join("scene.json"),
        scene.replace("systemfont_nonesuch", "fonts/absent.ttf"),
    )
    .unwrap();
    let refused = bare.prepare().err().unwrap().to_string();
    assert!(refused.contains("absent.ttf"), "{refused}");
    std::fs::remove_dir_all(folder).unwrap();
}
