use super::converted as with_combos;

fn converted(source: &str) -> String {
    with_combos(source, &std::collections::BTreeMap::new())
}

#[test]
fn vectors_and_numbers_are_made_what_they_set() {
    let source = "uniform vec4 c;\nvoid main() {\n float r = step(1.0, c);\n vec3 beam = r;\n vec2 cut = c;\n}";
    let out = converted(source);
    assert!(out.contains("float r = (step(1.0, c)).x;"), "{out}");
    assert!(out.contains("vec3 beam = vec3(r);"), "{out}");
    assert!(out.contains("vec2 cut = (c).xy;"), "{out}");
    assert_eq!(out.lines().count(), source.lines().count());
}

#[test]
fn whole_numbers_beside_fractions_are_made_fractions() {
    let source = "uniform float low;\nuniform float high;\nfloat f(float x) { return x * 2; }\nvoid main() {\n for (int i = low; i < high; i++) { float y = f(i) + pow(high, 2); }\n}";
    let out = converted(source);
    assert!(out.contains("return x * 2.0;"), "{out}");
    assert!(out.contains("int i = int(low);"), "{out}");
    assert!(out.contains("float(i) < high"), "{out}");
    assert!(out.contains("f(float(i))"), "{out}");
    assert!(out.contains("pow(high, 2.0)"), "{out}");
}

#[test]
fn a_remainder_of_fractions_is_mod_and_a_const_of_anything_isnt_const() {
    let source = "uniform vec4 c;\nvoid main() {\n float x = c.x % 3.0;\n const vec4 w = vec4(c.rgb, 0.0);\n const float k = 2.0 * 3.0;\n int n = 7 % 2;\n}";
    let out = converted(source);
    assert!(out.contains("float x = mod(c.x , 3.0);"), "{out}");
    assert!(out.contains("  vec4 w = vec4(c.rgb, 0.0);"), "{out}");
    assert!(out.contains("const float k = 2.0 * 3.0;"), "{out}");
    assert!(out.contains("int n = 7 % 2;"), "{out}");
}

#[test]
fn glsl_that_already_compiles_is_left_alone() {
    for source in [
        "uniform sampler2D g_Texture0;\nvarying vec2 v_TexCoord;\nvoid main() { vec4 c = texSample2D(g_Texture0, v_TexCoord); gl_FragColor = vec4(c.rgb * 0.5, c.a); }",
        "struct S { float a; };\nvec3 f(vec3 a, float b) { return mix(a, vec3(1.0), b); }\nvoid main() { S s; float k[3]; k[0] = 1.0; }",
        "#define N 4\nvoid main() { for (int i = 0; i < N; i++) {} }",
    ] {
        assert_eq!(converted(source), source);
    }
}

#[test]
fn combos_macros_unsigned_and_true_or_false_are_numbers_too() {
    let combos = std::collections::BTreeMap::from([("RESOLUTION".to_owned(), 32)]);
    let source = "#define SCALE 2.5\nvoid main() {\n float f = 3.5;\n bool on = f > 1.0;\n uint a = f % RESOLUTION;\n uint b = (a + 1) % RESOLUTION;\n float g = f * SCALE;\n f *= on;\n}";
    let out = with_combos(source, &combos);
    assert!(
        out.contains("uint a = uint(mod(f , float(RESOLUTION)));"),
        "{out}"
    );
    assert!(
        out.contains("uint b = (a + 1u) % uint(RESOLUTION);"),
        "{out}"
    );
    assert!(out.contains("float g = f * SCALE;"), "{out}");
    assert!(out.contains("f *= float(on);"), "{out}");
}

#[test]
fn hex_numbers_are_whole_even_with_an_f_in_them() {
    let source = "void main() {\n int n = 3;\n if (n == 0x1F) { n = 0xAF; }\n uint m = 0xFFu;\n}";
    assert_eq!(converted(source), source);
}
