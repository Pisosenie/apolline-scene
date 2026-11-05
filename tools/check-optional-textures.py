# GPU regression for optional mask combos and sparse sampler bindings.
import json
import math
from pathlib import Path
import runpy
import struct
import subprocess
import tempfile


def texture(fmt, width, pixels):
    header = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", fmt, 0, width, 1, width, 1, 0)
    return header + b"TEXB0002\0" + struct.pack("<7I", 1, 1, width, 1, 0, 0, len(pixels)) + pixels


def main():
    root = Path(__file__).resolve().parents[1]
    png = runpy.run_path(str(root / "tools/check-material-channels.py"))["png"]
    with tempfile.TemporaryDirectory(prefix="apolline-optional-") as temporary:
        folder = Path(temporary)
        (folder / "materials").mkdir()
        (folder / "shaders").mkdir()
        (folder / "materials/base.tex").write_bytes(texture(0, 1, bytes([200, 80, 40, 255])))
        (folder / "materials/mask.tex").write_bytes(texture(9, 2, bytes([0, 255])))
        (folder / "project.json").write_text("{}")
        (folder / "model.json").write_text('{"material":"material.json","fullscreen":true}')
        (folder / "scene.json").write_text('{"general":{"orthogonalprojection":{"width":64,"height":32},"clearcolor":"0.05 0.05 0.1"},"objects":[{"image":"model.json"}]}')
        frames = {}
        for slot in (1, 7):
            source = f'''varying vec4 v_TexCoord;
uniform sampler2D g_Texture0;
uniform sampler2D g_Texture{slot}; // {{"combo":"MASK","mode":"opacitymask"}}
uniform vec4 g_Texture{slot}Resolution;
#if MASK
uniform float g_Time;
#endif
void main() {{
    vec4 pixel = texSample2D(g_Texture0, v_TexCoord.xy);
#if MASK
    pixel.a *= texSample2D(g_Texture{slot}, v_TexCoord.xy).r * step(1.0, g_Texture{slot}Resolution.x) * (0.75 + 0.25 * cos(g_Time));
#endif
    gl_FragColor = pixel;
}}
'''
            (folder / "shaders/mask.frag").write_text(source)
            for bound in (False, True):
                for time in (0, 1):
                    names = ["base"] + [None] * slot
                    if bound:
                        names[slot] = "mask"
                    (folder / "material.json").write_text(json.dumps({"passes": [{"shader": "mask", "textures": names, "blending": "translucent"}]}))
                    shot = folder / f"{slot}-{bound}-{time}.png"
                    result = subprocess.run(["cargo", "run", "--offline", "--quiet", "--manifest-path", str(root / "Cargo.toml"), "--example", "view", "--", "--stats", "--shot-size", "64x32", "--time", str(time), "--shot", str(shot), str(folder)], capture_output=True, text=True)
                    if result.returncode:
                        raise RuntimeError(result.stderr)
                    assert f"{2 if bound else 1} texture uploads, {6 if bound else 4} logical texture bytes" in result.stderr, result.stderr
                    assert f"animated={str(bound).lower()}" in result.stderr, result.stderr
                    frame = png(shot)
                    assert frame[:3] == (64, 32, 4)
                    factor = 0.75 + 0.25*math.cos(time)
                    for x in (2, 61):
                        at = (16*64+x)*4
                        actual = tuple(frame[3][at:at+4])
                        clear, color = (13, 13, 26), (200, 80, 40)
                        alpha = (0 if x == 2 else factor) if bound else 1
                        expected = tuple(round(a*(1-alpha)+b*alpha) for a,b in zip(clear, color)) + (255,)
                        assert all(abs(a-b) <= 2 for a,b in zip(actual, expected)), (slot, bound, time, x, actual, expected)
                    frames[(slot, bound, time)] = frame
        assert frames[(1, False, 0)] == frames[(7, False, 1)]
        assert frames[(1, True, 0)] == frames[(7, True, 0)]
        assert frames[(1, True, 0)] != frames[(1, True, 1)]
        print("Optional masks: missing slots have one upload/no clock; bound slots 1 and 7 match, animate and blend correctly. No gap placeholders are uploaded.")


if __name__ == "__main__":
    main()
