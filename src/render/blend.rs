//! A layer's colour blend mode: its picture mixed with what's drawn under it, as image editors
//! mix layers, numbered as Wallpaper Engine numbers them. What's under it is a copy of the
//! frame so far, read where each pixel lands on screen.

/// The layer program's fragment shader for `mode`, 1 to 32.
pub(super) fn fragment(mode: u8) -> String {
    format!(
        "{HEAD}\nvec3 blended(vec3 a, vec3 b, float o) {{\n    {}\n}}\n{MAIN}",
        body(mode)
    )
}

/// How `b`, the layer, lands on `a`, what's under it, at opacity `o`.
fn body(mode: u8) -> &'static str {
    match mode {
        1 => "return mix(a, min(a, b), o);",
        2 => "return mix(a, a * b, o);",
        3 => "return mix(a, each(a, b, 3), o);",
        4 | 20 => "return mix(a, max(a + b - 1.0, 0.0), o);",
        5 => "return min(a, b);",
        6 => "return mix(a, max(a, b), o);",
        7 => "return mix(a, 1.0 - (1.0 - a) * (1.0 - b), o);",
        8 => "return mix(a, each(a, b, 8), o);",
        9 => "return mix(a, min(a + b, 1.0), o);",
        10 => "return max(a, b);",
        11 => "return mix(a, each(a, b, 11), o);",
        12 => "return mix(a, each(a, b, 12), o);",
        13 => "return mix(a, each(b, a, 11), o);",
        14 => "return mix(a, each(a, b, 14), o);",
        15 => "return mix(a, each(a, b, 15), o);",
        16 => "return mix(a, each(a, b, 16), o);",
        17 => "return mix(a, each(a, b, 17), o);",
        18 => "return mix(a, abs(a - b), o);",
        19 => "return mix(a, a + b - 2.0 * a * b, o);",
        21 => "return mix(a, each(a, b, 21), o);",
        22 => "return mix(a, each(b, a, 21), o);",
        23 => "return mix(a, min(a, b) - max(a, b) + 1.0, o);",
        24 => "return mix(a, (a + b) * 0.5, o);",
        25 => "return mix(a, 1.0 - abs(1.0 - a - b), o);",
        26 => "return mix(a, rgb(vec3(hsl(b).x, hsl(a).yz)), o);",
        27 => "vec3 under = hsl(a); return mix(a, rgb(vec3(under.x, hsl(b).y, under.z)), o);",
        28 => "return mix(a, rgb(vec3(hsl(b).xy, hsl(a).z)), o);",
        29 => "return mix(a, rgb(vec3(hsl(a).xy, hsl(b).z)), o);",
        30 => "return mix(a, vec3(max(a.r, max(a.g, a.b))) * b, o);",
        31 => "return a + b * o;",
        32 => "return mix(a, a + a * b, o);",
        _ => "return mix(a, b, o);",
    }
}

const HEAD: &str = "
uniform sampler2D picture;
uniform sampler2D under;
uniform vec2 apolline_screen;
uniform vec4 tint;
uniform float power;
uniform int picture_channels;
in vec2 uv;
out vec4 color;

float dodge(float a, float b) { return b >= 1.0 ? 1.0 : min(a / (1.0 - b), 1.0); }
float burn(float a, float b) { return b <= 0.0 ? 0.0 : max(1.0 - (1.0 - a) / b, 0.0); }
float vivid(float a, float b) { return b < 0.5 ? burn(a, 2.0 * b) : dodge(a, 2.0 * b - 1.0); }

// The modes that work a channel at a time.
float one(float a, float b, int mode) {
    if (mode == 3) return burn(a, b);
    if (mode == 8) return dodge(a, b);
    if (mode == 11) return a < 0.5 ? 2.0 * a * b : 1.0 - 2.0 * (1.0 - a) * (1.0 - b);
    if (mode == 12) {
        return b < 0.5 ? 2.0 * a * b + a * a * (1.0 - 2.0 * b)
                       : sqrt(a) * (2.0 * b - 1.0) + 2.0 * a * (1.0 - b);
    }
    if (mode == 14) return vivid(a, b);
    if (mode == 15) return b < 0.5 ? max(a + 2.0 * b - 1.0, 0.0) : a + 2.0 * b - 1.0;
    if (mode == 16) return b < 0.5 ? min(a, 2.0 * b) : max(a, 2.0 * b - 1.0);
    if (mode == 17) return vivid(a, b) < 0.5 ? 0.0 : 1.0;
    if (mode == 21) return b >= 1.0 ? 1.0 : min(a * a / (1.0 - b), 1.0);
    return b;
}

vec3 each(vec3 a, vec3 b, int mode) {
    return vec3(one(a.r, b.r, mode), one(a.g, b.g, mode), one(a.b, b.b, mode));
}

// Hue, saturation and lightness, each 0 to 1.
vec3 hsl(vec3 c) {
    float high = max(c.r, max(c.g, c.b));
    float low = min(c.r, min(c.g, c.b));
    float l = (high + low) * 0.5;
    float d = high - low;
    if (d <= 0.0) return vec3(0.0, 0.0, l);
    float s = l < 0.5 ? d / (high + low) : d / (2.0 - high - low);
    float h = high == c.r ? (c.g - c.b) / d + (c.g < c.b ? 6.0 : 0.0)
            : high == c.g ? (c.b - c.r) / d + 2.0
            : (c.r - c.g) / d + 4.0;
    return vec3(h / 6.0, s, l);
}

float channel(float p, float q, float h) {
    h = fract(h);
    if (h < 1.0 / 6.0) return p + (q - p) * 6.0 * h;
    if (h < 0.5) return q;
    if (h < 2.0 / 3.0) return p + (q - p) * (2.0 / 3.0 - h) * 6.0;
    return p;
}

vec3 rgb(vec3 c) {
    if (c.y <= 0.0) return vec3(c.z);
    float q = c.z < 0.5 ? c.z * (1.0 + c.y) : c.z + c.y - c.z * c.y;
    float p = 2.0 * c.z - q;
    return vec3(channel(p, q, c.x + 1.0 / 3.0), channel(p, q, c.x), channel(p, q, c.x - 1.0 / 3.0));
}
";

const MAIN: &str = "
void main() {
    vec4 pixel = texture(picture, uv);
    if (picture_channels == 1) {
        pixel = vec4(pixel.rrr, 1.0);
    } else if (picture_channels == 2) {
        pixel = vec4(pixel.rrr, pixel.g);
    }
    pixel *= tint;
    if (power != 1.0) {
        pixel.rgb = pow(max(pixel.rgb, 0.0), vec3(power));
    }
    vec3 below = texture(under, gl_FragCoord.xy / apolline_screen).rgb;
    color = vec4(blended(below, pixel.rgb, pixel.a), 1.0);
}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_its_own_sum() {
        let bodies: std::collections::HashSet<&str> = (1..=32).map(body).collect();
        // Subtract is listed twice.
        assert_eq!(bodies.len(), 31);
        assert!(fragment(30).contains("max(a.r, max(a.g, a.b))"));
    }
}
