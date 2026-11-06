#!/usr/bin/env bash
# Draws the demo scenes and a dialect material through the viewer and prints each frame's first
# pixel and a hash of all of them. Hashes only compare within one driver. On Linux that's GLES under a throwaway headless sway (Mesa's software GLES does); on
# macOS it's desktop OpenGL, so the two can be compared.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
runtime=$(mktemp -d)
chmod 700 "$runtime"
trap 'rm -rf "$work" "$runtime"' EXIT
if [ "$(uname)" = Linux ]; then
    printf 'output HEADLESS-1 resolution 1280x720 scale 1\n' > "$work/sway.conf"
    XDG_RUNTIME_DIR=$runtime WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 WLR_RENDERER=pixman \
        sway -c "$work/sway.conf" > /dev/null 2>&1 &
    sway=$!
    trap 'kill $sway 2> /dev/null; rm -rf "$work" "$runtime"' EXIT
    for _ in $(seq 50); do [ -S "$runtime/wayland-1" ] && break; sleep 0.1; done
    export XDG_RUNTIME_DIR=$runtime WAYLAND_DISPLAY=wayland-1 LIBGL_ALWAYS_SOFTWARE=1
fi
cargo build --quiet --example view --manifest-path "$root/Cargo.toml"
view=${CARGO_TARGET_DIR:-$root/target}/debug/examples/view
projects=(demo fragment mask wave vertex effect effect-script animation sprite pointer buffer composite values particle inputs script text dialect)
python3 "$root/tools/demo-scene.py" "$work/demo" > /dev/null
python3 "$root/tools/demo-scene.py" "$work/fragment" --fragment-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/mask" --mask-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/wave" --wave-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/vertex" --vertex-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/effect" --effect-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/effect-script" --effect-script-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/animation" --animation-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/sprite" --sprite-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/pointer" --pointer-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/buffer" --buffer-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/composite" --composite-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/compose" --compose-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/values" --values-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/particle" --particle-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/inputs" --inputs-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/script" --script-demo > /dev/null
python3 "$root/tools/demo-scene.py" "$work/text" --text-demo > /dev/null
# Wallpaper Engine's dialect: lerp, fmod (truncating, so this one comes out black), mul, and a
# helper with out parameters, on a red clear colour that shows if the material is refused.
mkdir -p "$work/dialect/shaders" "$work/dialect/materials"
python3 - "$work/dialect" <<'PY'
import json, struct, sys
from pathlib import Path
out = Path(sys.argv[1])
def tex(w, h, pixels):
    head = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", 0, 0, w, h, w, h, 0) + b"TEXB0002\0"
    return head + struct.pack("<2I", 1, 1) + struct.pack("<5I", w, h, 0, 0, len(pixels)) + pixels
(out / "project.json").write_text(json.dumps({"type": "scene", "file": "scene.json"}))
(out / "scene.json").write_text(json.dumps({"general": {"orthogonalprojection": {"width": 64, "height": 64}, "clearcolor": "1 0 0"}, "objects": [{"name": "x", "image": "model.json"}]}))
(out / "model.json").write_text(json.dumps({"material": "material.json", "fullscreen": True}))
(out / "material.json").write_text(json.dumps({"passes": [{"shader": "demo", "textures": ["white"], "blending": "normal", "cullmode": "nocull", "depthtest": "disabled", "depthwrite": "disabled"}]}))
(out / "materials/white.tex").write_bytes(tex(1, 1, bytes([255, 255, 255, 255])))
(out / "shaders/demo.vert").write_text("""attribute vec3 a_Position;
attribute vec2 a_TexCoord;
uniform mat4 g_ModelViewProjectionMatrix;
varying vec2 v_TexCoord;
void main() {
    gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix);
    v_TexCoord = a_TexCoord;
}
""")
(out / "shaders/demo.frag").write_text("""varying vec2 v_TexCoord;
uniform sampler2D g_Texture0;
void split(vec4 c, out vec3 rgb, out float a) { rgb = c.rgb; a = c.a; }
void main() {
    vec3 rgb; float a;
    split(texSample2D(g_Texture0, v_TexCoord), rgb, a);
    vec4 turned = mul(vec4(rgb, 1.0), mat4(1.0));
    gl_FragColor = vec4(lerp(turned.rgb, vec3(fmod(-3.0, 2.0)), 0.5), a);
}
""")
PY
first_pixel() {
    python3 - "$1" <<'PY'
import struct, sys, zlib
data = open(sys.argv[1], "rb").read()
at, packed = 8, b""
while at < len(data):
    size = struct.unpack(">I", data[at:at + 4])[0]
    kind, body = data[at + 4:at + 8], data[at + 8:at + 8 + size]
    at += 12 + size
    if kind == b"IHDR":
        color = body[9]
    if kind == b"IDAT":
        packed += body
import hashlib
raw = zlib.decompress(packed)
print(list(raw[1:1 + (4 if color == 6 else 3)]), "all", hashlib.sha1(raw).hexdigest()[:12])
PY
}
# The largest difference in any channel between two shots of one size.
difference() {
    python3 - "$1" "$2" <<'PY'
import struct, sys, zlib
def pixels(path):
    data = open(path, "rb").read()
    at, packed = 8, b""
    while at < len(data):
        size = struct.unpack(">I", data[at:at + 4])[0]
        kind, body = data[at + 4:at + 8], data[at + 8:at + 8 + size]
        at += 12 + size
        if kind == b"IHDR":
            width, height, color = struct.unpack(">I", body[:4])[0], struct.unpack(">I", body[4:8])[0], body[9]
        if kind == b"IDAT":
            packed += body
    depth = 4 if color == 6 else 3
    raw, stride, rows, above = zlib.decompress(packed), width * depth, [], bytearray(width * depth)
    for row in range(height):
        kind, line = raw[row * (stride + 1)], bytearray(raw[row * (stride + 1) + 1:(row + 1) * (stride + 1)])
        for x in range(stride):
            left = line[x - depth] if x >= depth else 0
            up, corner = above[x], above[x - depth] if x >= depth else 0
            if kind == 1:
                line[x] = (line[x] + left) & 255
            elif kind == 2:
                line[x] = (line[x] + up) & 255
            elif kind == 3:
                line[x] = (line[x] + (left + up) // 2) & 255
            elif kind == 4:
                guess = left + up - corner
                near = min((abs(guess - left), left), (abs(guess - up), up), (abs(guess - corner), corner), key=lambda pair: pair[0])
                line[x] = (line[x] + near[1]) & 255
        rows.append(line)
        above = line
    return b"".join(rows)
a, b = pixels(sys.argv[1]), pixels(sys.argv[2])
print(max(abs(x - y) for x, y in zip(a, b)) if len(a) == len(b) else 255)
PY
}
for project in "${projects[@]}"; do
    # Timelines are shot a second in, when they've put everything back, and the sprite sheet
    # on its last frame.
    when=(); [ "$project" = animation ] && when=(--time 1)
    [ "$project" = sprite ] && when=(--time 1.25)
    "$view" --shot "$work/$project.png" ${when[@]+"${when[@]}"} "$work/$project" 2>&1 | sed "s/^/$project: /"
    pixel=$(first_pixel "$work/$project.png")
    echo "${pixel##* }" > "$work/$project.hash"
    echo "$project: first pixel $pixel"
done
# Hashes differ between drivers, but these demos have to draw what the plain one does on each.
for project in vertex effect effect-script animation pointer buffer composite inputs script; do
    cmp -s "$work/$project.hash" "$work/demo.hash" && echo "$project: same as demo" || echo "$project: DIFFERS from demo"
done
# A composition layer draws the frame at the scene's size, so it's shot at that size, against
# the plain demo shot so too.
"$view" --shot "$work/compose.png" --shot-size 1920x1080 "$work/compose" 2>&1 | sed "s/^/compose: /"
"$view" --shot "$work/demo-1080.png" --shot-size 1920x1080 "$work/demo" > /dev/null 2>&1
[ "$(difference "$work/compose.png" "$work/demo-1080.png")" -eq 0 ] && echo "compose: same as demo" || echo "compose: DIFFERS from demo"
# A frame read from inside a sprite sheet rounds its way to the picture a little differently.
[ "$(difference "$work/sprite.png" "$work/demo.png")" -le 1 ] && echo "sprite: within one of demo" || echo "sprite: DIFFERS from demo"
