# Writes a small made-up scene project to the folder given: a sky, a turned translucent sun, an
# additive grey glow, a DXT1 brick and a hidden layer, in the formats as apolline-scene reads
# them. Draw it with `cargo run --example view -- --shot frame.png <folder>`.
import json, math, struct, sys
out = sys.argv[1]
def tex(fmt, w, h, pixels):
    b = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", fmt, 0, w, h, w, h, 0) + b"TEXB0002\0"
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
                    "cameraparallax": True, "cameraparallaxamount": 0.5},
        "objects": [
            {"name": "sky", "image": "models/sky.json"},
            {"name": "sun", "image": "models/sun.json", "origin": "1400 800 0", "size": "300 300", "angles": "0 0 0.3"},
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
def string(t): t = t.encode(); return struct.pack("<I", len(t)) + t
table, data, offset = b"", b"", 0
for name, body in files.items():
    table += string(name) + struct.pack("<II", offset, len(body)); offset += len(body); data += body
open(f"{out}/scene.pkg", "wb").write(string("PKGV0021") + struct.pack("<I", len(files)) + table + data)
open(f"{out}/project.json", "w").write(json.dumps({"title": "Demo", "type": "scene", "file": "scene.json"}))
