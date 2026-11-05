# Writes a small made-up scene project to the folder given: a sky, a turned translucent sun, an
# additive grey glow, a DXT1 brick and a hidden layer, in the formats as apolline-scene reads
# them. Draw it with `cargo run --example view -- --shot frame.png <folder>`.
import argparse, json, math, struct
from pathlib import Path

parser = argparse.ArgumentParser(description="Write a synthetic Wallpaper Engine scene")
parser.add_argument("folder")
parser.add_argument("--still", action="store_true", help="omit pointer parallax for native daemon playback")
demos = parser.add_mutually_exclusive_group()
demos.add_argument("--fragment-demo", action="store_true", help="apply a synthetic static desaturation material to the sky")
demos.add_argument("--mask-demo", action="store_true", help="apply a synthetic two-texture opacity mask")
parser.add_argument("--mask-format", choices=("r", "rg", "rgba"), default="r", help="opacity-mask storage (default: R8)")
demos.add_argument("--wave-demo", action="store_true", help="apply a synthetic native animated wave material")
demos.add_argument("--vertex-demo", action="store_true", help="draw the sky and sun through pass-through materials with their own vertex shaders")
demos.add_argument("--effect-demo", action="store_true", help="put effects on the sky and sun that undo each other, so it draws what the plain demo draws")
demos.add_argument("--values-demo", action="store_true", help="draw the glow green and the brick blue through one material with different values")
demos.add_argument("--particle-demo", action="store_true", help="add rising embers, already two seconds in")
demos.add_argument("--inputs-demo", action="store_true", help="read the pointer, parallax, daytime and audio at their resting values, which leave the sky as it is")
parser.add_argument("--without-mask", action="store_true", help="leave the mask sampler unbound in --mask-demo")
args = parser.parse_args()
if args.without_mask and not args.mask_demo:
    parser.error("--without-mask requires --mask-demo")
out = args.folder
Path(out).mkdir(parents=True, exist_ok=True)
def tex(fmt, w, h, pixels, flags=2):
    # Flag 2 clamps at the edges, as pictures do; tiling textures leave it out.
    b = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", fmt, flags, w, h, w, h, 0) + b"TEXB0002\0"
    return b + struct.pack("<I", 1) + struct.pack("<I", 1) + struct.pack("<5I", w, h, 0, 0, len(pixels)) + pixels
sky = bytes([40, 90, 200, 255] * 2 + [250, 150, 60, 255] * 2)  # top row blue, bottom row orange
sun = bytearray()
for y in range(16):
    for x in range(16):
        r = math.hypot(x - 7.5, y - 7.5) / 8
        sun += bytes([255, 230, 120, max(0, min(255, int((1 - r) * 400)))])
grey = bytes(int(255 * (1 - abs(x - 3.5) / 4)) for y in range(4) for x in range(8))
red_block = struct.pack("<HHI", 0xF800, 0xF800, 0)  # DXT1, both endpoints pure red
files = {
    "scene.json": json.dumps({
        "general": {"clearcolor": "0.05 0.05 0.1", "orthogonalprojection": {"width": 1920, "height": 1080},
                    "cameraparallax": not args.still, "cameraparallaxamount": 0.5},
        "objects": [
            {"name": "sky", "image": "models/sky.json"},
            {"name": "sun", "image": "models/sun.json", "origin": "1400 800 0", "size": "300 300", "angles": "0 0 0.3", "visible": {"user": "show_sun", "value": True}, "alpha": {"user": "sun_opacity", "value": 1}, "color": {"user": "sun_color", "value": "1 1 1"}},
            {"name": "glow", "image": "models/glow.json", "origin": "500 300 0", "size": "400 200", "alpha": 0.8},
            {"name": "brick", "image": "models/brick.json", "origin": "960 150 0", "size": "200 100", "color": "1 1 1"},
            {"name": "hidden", "image": "models/brick.json", "origin": "960 540 0", "size": "800 800", "visible": {"user": "x", "value": False}},
        ]}).encode(),
    "models/sky.json": b'{"material": "materials/sky.json", "fullscreen": true}',
    "materials/sky.json": b'{"passes": [{"textures": ["sky"], "blending": "normal"}]}',
    "materials/sky.tex": tex(0, 2, 2, sky),
    "models/sun.json": b'{"material": "materials/sun.json"}',
    "materials/sun.json": b'{"passes": [{"textures": ["sun"], "blending": "translucent"}]}',
    "materials/sun.tex": tex(0, 16, 16, bytes(sun)),
    "models/glow.json": b'{"material": "materials/glow.json"}',
    "materials/glow.json": b'{"passes": [{"textures": ["glow"], "blending": "additive"}]}',
    "materials/glow.tex": tex(9, 8, 4, grey),
    "models/brick.json": b'{"material": "materials/brick.json"}',
    "materials/brick.json": b'{"passes": [{"textures": ["brick"], "blending": "normal"}]}',
    "materials/brick.tex": tex(7, 4, 4, red_block),
}
if args.fragment_demo:
    files["materials/sky.json"] = json.dumps({"passes": [{"shader": "demo/desaturate", "textures": ["sky"], "blending": "normal", "combos": {"GRAY": 1}, "constantshadervalues": {"amount": {"user": "effect_amount", "value": 1}}}]}).encode()
    files["shaders/demo/tone.h"] = b"float luminance_value(vec3 color) { return dot(color, vec3(0.2126, 0.7152, 0.0722)); }\n"
    files["shaders/demo/desaturate.frag"] = b'''// [COMBO] {"combo":"GRAY","default":0}
#include "tone.h"
varying vec4 v_TexCoord;
uniform sampler2D g_Texture0;
uniform float u_Amount; // {"material":"amount","default":1}
void main() {
    vec4 pixel = texSample2D(g_Texture0, v_TexCoord.xy);
#if GRAY
    pixel.rgb = mix(pixel.rgb, vec3(luminance_value(pixel.rgb)), u_Amount);
#endif
    gl_FragColor = pixel;
}
'''
if args.mask_demo:
    files["materials/sky.json"] = json.dumps({"passes":[{"shader":"demo/mask", "textures":["sky", None if args.without_mask else "opacity-mask"], "blending":"translucent", "constantshadervalues":{"amount":{"user":"mask_amount", "value":1}}}]}).encode()
    mask_format, mask_pixels = {
        "r": (9, bytes([0, 255])),
        "rg": (8, bytes([0, 255, 255, 255])),
        "rgba": (0, bytes([0, 0, 0, 255, 255, 255, 255, 255])),
    }[args.mask_format]
    files["materials/opacity-mask.tex"] = tex(mask_format, 2, 1, mask_pixels)
    files["shaders/demo/mask.frag"] = b'''varying vec4 v_TexCoord;
uniform sampler2D g_Texture0;
uniform sampler2D g_Texture1; // {"combo":"MASK","mode":"opacitymask"}
uniform vec4 g_Texture1Resolution;
uniform float u_Amount; // {"material":"amount","default":1}
void main() {
    vec4 pixel = texSample2D(g_Texture0, v_TexCoord.xy);
#if MASK
    float mask = texSample2D(g_Texture1, v_TexCoord.xy).r * step(1.0, g_Texture1Resolution.x);
    pixel.a *= mix(1.0, mask, u_Amount);
#endif
    gl_FragColor = pixel;
}
'''
if args.wave_demo:
    files["materials/sky.json"] = json.dumps({"passes":[{"shader":"demo/wave", "textures":["sky"], "blending":"normal", "combos":{"WAVE":{"user":"wave_enabled", "value":True}}, "constantshadervalues":{"strength":{"user":"wave_strength", "value":0.08}, "speed":{"user":"wave_speed", "value":1.5}}}]}).encode()
    files["shaders/demo/wave.frag"] = b'''// [COMBO] {"combo":"WAVE","default":1}
varying vec4 v_TexCoord;
uniform sampler2D g_Texture0;
uniform float u_Strength; // {"material":"strength","default":0.08}
uniform float u_Speed; // {"material":"speed","default":1.5}
#if WAVE
uniform float g_Time;
#endif
void main() {
    vec2 uv = v_TexCoord.xy;
#if WAVE
    uv.y = clamp(uv.y + sin(uv.x * 12.0 - g_Time * u_Speed) * u_Strength, 0.0, 1.0);
#endif
    gl_FragColor = texSample2D(g_Texture0, uv);
}
'''
if args.vertex_demo:
    # Pass-through materials with their own vertex shaders draw what the plain layers draw.
    files["materials/sky.json"] = b'{"passes": [{"shader": "demo/through", "textures": ["sky"], "blending": "normal"}]}'
    files["materials/sun.json"] = b'{"passes": [{"shader": "demo/through", "textures": ["sun"], "blending": "translucent"}]}'
    files["shaders/demo/through.vert"] = b'''attribute vec3 a_Position;
attribute vec2 a_TexCoord;
uniform mat4 g_ModelViewProjectionMatrix;
varying vec4 v_TexCoord;
varying float v_Keep;
void main() {
    gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix);
    v_TexCoord = vec4(a_TexCoord * 2.0, a_TexCoord);
    v_Keep = 1.0;
}
'''
    files["shaders/demo/through.frag"] = b'''varying vec4 v_TexCoord;
varying float v_Keep;
uniform sampler2D g_Texture0;
void main() {
    gl_FragColor = texSample2D(g_Texture0, v_TexCoord.zw) * v_Keep;
}
'''
if args.effect_demo:
    # Two inversions on the sun, the second through its own vertex shader, and a tint on the
    # sky that only the instance's own value and a white second texture leave as it was.
    scene = json.loads(files["scene.json"])
    # The sky's chain has an odd number of draws, so a flip in each would show.
    scene["objects"][0]["effects"] = [{"file": "effects/tint/effect.json", "passes": [{"constantshadervalues": {"color": "1 1 1"}}]}, {"file": "effects/copy/effect.json"}]
    scene["objects"][1]["effects"] = [{"file": "effects/invert/effect.json"}, {"file": "effects/invert/effect.json", "passes": [{"combos": {"STAGED": 1}}]}]
    files["scene.json"] = json.dumps(scene).encode()
    files["effects/invert/effect.json"] = b'{"name": "invert", "passes": [{"material": "materials/effects/invert.json"}]}'
    files["materials/effects/invert.json"] = b'{"passes": [{"shader": "effects/invert", "blending": "normal", "textures": [null]}]}'
    files["shaders/effects/invert.vert"] = b'''attribute vec3 a_Position;
attribute vec2 a_TexCoord;
uniform mat4 g_ModelViewProjectionMatrix;
varying vec2 v_TexCoord;
void main() {
    gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix);
    // STAGED is the fragment's combo, on by default; without it the picture would go.
#if STAGED
    v_TexCoord = a_TexCoord;
#else
    v_TexCoord = vec2(0.0);
#endif
}
'''
    files["shaders/effects/invert.frag"] = b'''// [COMBO] {"combo":"STAGED","default":1}
varying vec2 v_TexCoord;
uniform sampler2D g_Texture0;
void main() {
    vec4 pixel = texSample2D(g_Texture0, v_TexCoord);
    gl_FragColor = vec4(1.0 - pixel.rgb, pixel.a);
}
'''
    files["effects/copy/effect.json"] = b'{"name": "copy", "passes": [{"material": "materials/effects/copy.json"}]}'
    files["materials/effects/copy.json"] = b'{"passes": [{"shader": "effects/copy", "textures": [null]}]}'
    files["shaders/effects/copy.frag"] = b"varying vec2 v_TexCoord;\nuniform sampler2D g_Texture0;\nvoid main() { gl_FragColor = texSample2D(g_Texture0, v_TexCoord); }\n"
    files["effects/tint/effect.json"] = b'{"name": "tint", "passes": [{"material": "materials/effects/tint.json"}]}'
    files["materials/effects/tint.json"] = b'{"passes": [{"shader": "effects/tint", "textures": [null, "util/white"], "constantshadervalues": {"color": "0 0 0"}}]}'
    files["materials/util/white.tex"] = tex(0, 1, 1, bytes([255, 255, 255, 255]))
    files["shaders/effects/tint.frag"] = b'''varying vec2 v_TexCoord;
uniform sampler2D g_Texture0;
uniform sampler2D g_Texture1;
uniform vec3 u_Color; // {"material":"color","default":"1 1 1"}
void main() {
    gl_FragColor = texSample2D(g_Texture0, v_TexCoord) * vec4(u_Color, 1.0) * texSample2D(g_Texture1, v_TexCoord);
}
'''
if args.values_demo:
    # One program, two layers: each must come out in its own colour.
    files["materials/glow.json"] = b'{"passes": [{"shader": "demo/solid", "textures": ["glow"], "blending": "normal", "constantshadervalues": {"color": "0 1 0"}}]}'
    files["materials/brick.json"] = b'{"passes": [{"shader": "demo/solid", "textures": ["brick"], "blending": "normal", "constantshadervalues": {"color": "0 0 1"}}]}'
    files["shaders/demo/solid.frag"] = b"uniform sampler2D g_Texture0;\nuniform vec3 u_Color; // {\"material\":\"color\",\"default\":\"1 0 0\"}\nvoid main() { gl_FragColor = vec4(u_Color, 1.0); }\n"
if args.particle_demo:
    scene = json.loads(files["scene.json"])
    scene["objects"].insert(3, {"name": "embers", "particle": "particles/embers.json", "origin": "960 100 0", "scale": "1 1 1"})
    files["scene.json"] = json.dumps(scene).encode()
    files["particles/embers.json"] = json.dumps({
        "material": "materials/particle/ember.json", "maxcount": 200, "starttime": 2,
        "emitter": [{"name": "boxrandom", "rate": 60, "distancemax": "700 20 0"}],
        "initializer": [{"name": "lifetimerandom", "min": 2, "max": 4}, {"name": "sizerandom", "min": 10, "max": 30},
                        {"name": "colorrandom", "min": "255 120 40", "max": "255 220 120"},
                        {"name": "velocityrandom", "min": "-20 120 0", "max": "20 260 0"}],
        "operator": [{"name": "movement", "gravity": "0 -20 0", "drag": 0.1},
                     {"name": "alphafade", "fadeintime": 0.1, "fadeouttime": 0.6},
                     {"name": "sizechange", "startvalue": 1, "endvalue": 0.3}],
        "renderer": [{"name": "sprite"}]}).encode()
    files["materials/particle/ember.json"] = b'{"passes": [{"shader": "genericparticle", "textures": ["particle/dot"], "blending": "additive"}]}'
    dot = bytearray()
    for y in range(16):
        for x in range(16):
            r = math.hypot(x - 7.5, y - 7.5) / 8
            dot += bytes([255, 255, 255, max(0, min(255, int((1 - r) * 255)))])
    files["materials/particle/dot.tex"] = tex(0, 16, 16, bytes(dot))
if args.inputs_demo:
    files["materials/sky.json"] = b'{"passes": [{"shader": "demo/inputs", "textures": ["sky"], "blending": "normal"}]}'
    files["shaders/demo/inputs.frag"] = b"""varying vec2 v_TexCoord;
uniform sampler2D g_Texture0;
uniform vec2 g_PointerPosition;
uniform vec2 g_ParallaxPosition;
uniform float g_Daytime;
uniform float g_AudioSpectrum16Left[16];
uniform float g_AudioSpectrum64Right[64];
void main() {
    float one = g_PointerPosition.x * 2.0 * g_ParallaxPosition.y * 2.0 * g_Daytime * 2.0;
    one *= 1.0 - g_AudioSpectrum16Left[3] - g_AudioSpectrum64Right[60];
    gl_FragColor = texSample2D(g_Texture0, v_TexCoord) * vec4(vec3(one), 1.0);
}
"""
def string(t): t = t.encode(); return struct.pack("<I", len(t)) + t
table, data, offset = b"", b"", 0
for name, body in files.items():
    table += string(name) + struct.pack("<II", offset, len(body)); offset += len(body); data += body
open(f"{out}/scene.pkg", "wb").write(string("PKGV0021") + struct.pack("<I", len(files)) + table + data)
open(f"{out}/project.json", "w").write(json.dumps({"title": "Demo", "type": "scene", "file": "scene.json", "general": {"properties": {
    "show_sun": {"type": "bool", "text": "Show sun", "value": True, "order": 0},
    "sun_opacity": {"type": "slider", "text": "Sun opacity", "value": 1, "min": 0, "max": 1, "fraction": True, "order": 1},
    "sun_color": {"type": "color", "text": "Sun color", "value": "1 1 1", "order": 2},
}}}))

if args.fragment_demo:
    metadata = json.loads(Path(out, "project.json").read_text())
    metadata["general"]["properties"]["effect_amount"] = {"type":"slider", "text":"Desaturation", "value":1, "min":0, "max":1, "fraction":True, "order":3}
    Path(out, "project.json").write_text(json.dumps(metadata))

if args.mask_demo:
    metadata = json.loads(Path(out, "project.json").read_text())
    metadata["general"]["properties"]["mask_amount"] = {"type":"slider", "text":"Opacity mask", "value":1, "min":0, "max":1, "fraction":True, "order":3}
    Path(out, "project.json").write_text(json.dumps(metadata))

if args.wave_demo:
    metadata = json.loads(Path(out, "project.json").read_text())
    metadata["general"]["properties"].update({
        "wave_enabled": {"type":"bool", "text":"Animate sky", "value":True, "order":3},
        "wave_strength": {"type":"slider", "text":"Wave strength", "value":0.08, "min":0,"max":0.2,"fraction":True,"order":4},
        "wave_speed": {"type":"slider", "text":"Wave speed", "value":1.5,"min":0,"max":3,"fraction":True,"order":5},
    })
    Path(out, "project.json").write_text(json.dumps(metadata))
