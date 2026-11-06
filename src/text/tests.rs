use super::*;
use crate::scene::Image;

const FONT: &[u8] = include_bytes!("../../tests/fonts/Cantarell-Regular.ttf");

fn words(text: &str) -> Text {
    Text {
        layer: Image {
            name: "words".into(),
            model: String::new(),
            origin: [0.0; 3],
            scale: [1.0; 3],
            angles: [0.0; 3],
            size: None,
            color: [1.0; 3],
            alpha: 1.0,
            brightness: 1.0,
            anchor: [0.0; 2],
            visible: true,
            depth: [1.0; 2],
            effects: Vec::new(),
            instance: None,
            mode: 0,
        },
        text: text.into(),
        font: "fonts/Cantarell-Regular.ttf".into(),
        points: 12.0,
        align: [0.0; 2],
        padding: [0.0; 2],
        width: None,
        rows: None,
        ellipsis: false,
    }
}

fn lettering(text: &Text) -> Lettering {
    // Effects keep a texel to a scene pixel, which the sizes below are in.
    Lettering::new(parse(FONT.to_vec()).unwrap(), Vec::new(), text, true)
}

fn coverage(raster: &Raster) -> Vec<u8> {
    raster.texture.images[0][0]
        .pixels
        .chunks(2)
        .map(|pixel| pixel[1])
        .collect()
}

#[test]
fn words_are_as_tall_as_their_lines_at_300_to_the_inch() {
    let text = words("Hello\nworld");
    let lettering = lettering(&text);
    let (_, line) = lettering.metrics();
    let raster = lettering.raster(&text.text);
    // 12 points are 50 scene pixels to the em.
    assert!((lettering.em - 50.0).abs() < 1e-4);
    assert!(
        (raster.size.1 - (2.0 * line * 50.0).ceil()).abs() < 1e-3,
        "{:?}",
        raster.size
    );
    let one = lettering.raster("Hello");
    assert!(one.size.1 < raster.size.1 && one.size.0 > 50.0);
    assert!(coverage(&raster).iter().any(|&cover| cover > 200));
    let pixel = &raster.texture.images[0][0].pixels[..2];
    assert_eq!(
        pixel[0], 255,
        "words are white for the layer's colour to tint"
    );
    // Mipmaps go down to one texel.
    let last = raster.texture.images[0].last().unwrap();
    assert_eq!((last.width, last.height), (1, 1));
}

#[test]
fn padding_is_clear_room_and_the_origin_is_on_the_aligned_side() {
    let text = Text {
        padding: [10.0, 20.0],
        align: [-1.0, 1.0],
        ..words("Hi")
    };
    let lettering = lettering(&text);
    let raster = lettering.raster("Hi");
    let bare = Lettering::new(
        parse(FONT.to_vec()).unwrap(),
        Vec::new(),
        &Text {
            padding: [0.0; 2],
            ..text.clone()
        },
        true,
    )
    .raster("Hi");
    assert!((raster.size.0 - bare.size.0 - 20.0).abs() <= 1.0);
    assert!((raster.size.1 - bare.size.1 - 40.0).abs() <= 1.0);
    // The origin is at the words' top left corner, inside the padding.
    assert!((raster.anchor[0] - (-0.5 + 10.0 / raster.size.0)).abs() < 1e-5);
    assert!((raster.anchor[1] - (0.5 - 20.0 / raster.size.1)).abs() < 1e-5);
    let (width, height) = (
        raster.texture.width as usize,
        raster.texture.height as usize,
    );
    let cover = coverage(&raster);
    let empty = |rows: std::ops::Range<usize>, columns: std::ops::Range<usize>| {
        rows.into_iter()
            .all(|y| columns.clone().all(|x| cover[y * width + x] == 0))
    };
    assert!(empty(0..height, 0..9) && empty(0..19, 0..width));
}

#[test]
fn lines_wrap_at_the_widest_and_stop_at_the_most_rows() {
    let text = Text {
        width: Some(200.0),
        ..words("one two three four five six")
    };
    let lettering = lettering(&text);
    let lines = lettering.lines(&text.text);
    assert!(lines.len() > 1);
    for line in &lines {
        let (_, width) = lettering.places(line);
        assert!(width * lettering.em <= 200.0 + 1e-3);
        assert!(line.first().is_some_and(|letter| letter.character != ' '));
        assert!(line.last().is_some_and(|letter| letter.character != ' '));
    }
    let spelled: Vec<String> = lines
        .iter()
        .map(|line| line.iter().map(|letter| letter.character).collect())
        .collect();
    assert_eq!(spelled.join(" "), text.text);
    // A word longer than a line breaks inside itself, and every piece fits.
    for words in [
        "abcdefghijklmnopqrstuvwxyz",
        " abcdefghijklmnopqrstuvwxyz",
        "ab abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz",
    ] {
        let long = lettering.lines(words);
        assert!(long.len() > 1);
        for line in &long {
            assert!(!line.is_empty(), "no blank line in {words:?}");
            assert!(lettering.places(line).1 * lettering.em <= 200.0 + 1e-3);
        }
    }
    let cut = Text {
        rows: Some(1),
        ellipsis: true,
        ..text.clone()
    };
    let cut_lines = self::lettering(&cut).lines(&text.text);
    assert_eq!(cut_lines.len(), 1);
    let last: String = cut_lines[0].iter().map(|letter| letter.character).collect();
    assert!(last.ends_with('…') || last.ends_with("..."), "{last}");
    let (_, width) = self::lettering(&cut).places(&cut_lines[0]);
    assert!(width * lettering.em <= 200.0 + 1e-3);
}

#[test]
fn nothing_to_say_is_a_clear_texel() {
    let text = words("");
    let raster = lettering(&text).raster("");
    assert_eq!((raster.texture.width, raster.texture.height), (1, 1));
    assert_eq!(coverage(&raster), [0]);
    // Missing glyphs are the font's sign for one, not nothing.
    let missing = lettering(&text).raster("\u{e000}");
    assert!(missing.size.0 > 1.0);
}
