# GPU regression for compact textures shared by image layers and fragment materials.
# Run on a graphical macOS/Linux session: python3 tools/check-material-channels.py
import json
import subprocess
import sys
import tempfile

import struct,zlib
from pathlib import Path

def png(path):
    data = Path(path).read_bytes()
    offset = 8
    packed = bytearray()
    while offset < len(data):
        size = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        body = data[offset + 8:offset + 8 + size]
        offset += 12 + size
        if kind == b"IHDR":
            width, height, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            packed.extend(body)
    assert depth == 8 and color in (2, 6) and interlace == 0
    channels = 4 if color == 6 else 3
    stride = width * channels
    raw = zlib.decompress(packed)
    prior = bytearray(stride)
    pixels = bytearray()
    offset = 0
    for _ in range(height):
        mode = raw[offset]
        offset += 1
        row = bytearray(raw[offset:offset + stride])
        offset += stride
        for index in range(stride):
            left = row[index - channels] if index >= channels else 0
            above = prior[index]
            corner = prior[index - channels] if index >= channels else 0
            if mode == 0:
                predictor = 0
            elif mode == 1:
                predictor = left
            elif mode == 2:
                predictor = above
            elif mode == 3:
                predictor = (left + above) // 2
            else:
                assert mode == 4
                estimate = left + above - corner
                candidates = (left, above, corner)
                predictor = min(candidates, key=lambda value: abs(estimate - value))
            row[index] = (row[index] + predictor) & 255
        pixels.extend(row)
        prior = row
    return width, height, channels, pixels


def fixture(folder, channels, reverse):
    fmt, pixels = (9, bytes([64])) if channels == 1 else (8, bytes([64, 128]))
    texture = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", fmt, 0, 1, 1, 1, 1, 0)
    texture += b"TEXB0002\0" + struct.pack("<7I", 1, 1, 1, 1, 0, 0, len(pixels)) + pixels
    (folder / "materials").mkdir()
    (folder / "shaders").mkdir()
    (folder / "materials/shared.tex").write_bytes(texture)
    (folder / "project.json").write_text('{}')
    objects = []
    kinds = ["image", "raw", "converted"]
    if reverse:
        kinds.reverse()
    for index, kind in enumerate(kinds):
        objects.append({"image": f"{kind}.json", "origin": f"{index * 640 + 320} 540 0", "size": "640 1080"})
        (folder / f"{kind}.json").write_text(json.dumps({"material": f"materials/{kind}.json"}))
        shader = "genericimage" if kind == "image" else kind
        (folder / f"materials/{kind}.json").write_text(json.dumps({"passes": [{"shader": shader, "textures": ["shared"], "blending": "translucent"}]}))
    (folder / "scene.json").write_text(json.dumps({"general": {"orthogonalprojection": {"width": 1920, "height": 1080}}, "objects": objects}))
    prefix = "varying vec2 v_TexCoord;\nuniform sampler2D g_Texture0;\nvoid main() { vec4 pixel = texSample2D(g_Texture0, v_TexCoord); "
    (folder / "shaders/raw.frag").write_text(prefix + "gl_FragColor = pixel; }")
    alpha = "1.0" if channels == 1 else "pixel.g"
    (folder / "shaders/converted.frag").write_text(prefix + f"gl_FragColor = vec4(pixel.rrr, {alpha}); }}")
    return kinds


def main():
    root = Path(__file__).resolve().parents[1]
    # Compile once; the viewer creates an offscreen GL context for each fixture.
    built = subprocess.run(["cargo", "build", "--offline", "--quiet", "--manifest-path", str(root / "Cargo.toml"), "--example", "view"])
    if built.returncode:
        return built.returncode
    with tempfile.TemporaryDirectory(prefix="apolline-channels-") as temporary:
        temporary = Path(temporary)
        for channels in (1, 2):
            for reverse in (False, True):
                folder = temporary / f"r{channels}-{reverse}"
                folder.mkdir()
                kinds = fixture(folder, channels, reverse)
                shot = folder / "frame.png"
                ran = subprocess.run(["cargo", "run", "--offline", "--quiet", "--manifest-path", str(root / "Cargo.toml"), "--example", "view", "--", "--stats", "--shot", str(shot), str(folder)], capture_output=True, text=True)
                if ran.returncode:
                    raise RuntimeError(ran.stderr)
                assert f"1 texture uploads, {channels} logical texture bytes" in ran.stderr, ran.stderr
                w, h, count, pixels = png(shot)
                expected_image = (64, 64, 64, 255) if channels == 1 else (32, 32, 32, 255)
                expected_raw = (64, 0, 0, 255) if channels == 1 else (64, 128, 0, 255)
                for index, kind in enumerate(kinds):
                    expected = expected_raw if kind == "raw" else expected_image
                    for y in (100, 360, 600):
                        x = int((index + 0.5) * w / 3)
                        at = (y * w + x) * count
                        actual = tuple(pixels[at:at + count])
                        assert count == 4 and all(abs(a-b) <= 1 for a,b in zip(actual, expected)), (channels, reverse, kind, actual, expected)
                print(f"{'R8' if channels == 1 else 'RG8'}: raw sampling and image conversion match; one {channels}-byte upload, order={kinds}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
