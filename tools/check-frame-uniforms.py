# GPU regression of screen/texel uniforms at several offscreen framebuffer sizes.
import json
from pathlib import Path
import runpy
import struct
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parents[1]
    png = runpy.run_path(str(root / "tools/check-material-channels.py"))["png"]
    with tempfile.TemporaryDirectory(prefix="apolline-frame-") as temporary:
        folder = Path(temporary)
        (folder / "materials").mkdir()
        (folder / "shaders").mkdir()
        texture = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", 0, 0, 1, 1, 1, 1, 0)
        texture += b"TEXB0002\0" + struct.pack("<7I", 1, 1, 1, 1, 0, 0, 4) + bytes([255] * 4)
        (folder / "materials/shared.tex").write_bytes(texture)
        (folder / "project.json").write_text("{}")
        (folder / "model.json").write_text('{"material":"material.json","fullscreen":true}')
        (folder / "scene.json").write_text('{"general":{"orthogonalprojection":{"width":1280,"height":720}},"objects":[{"image":"model.json"}]}')
        (folder / "shaders/frame.frag").write_text('''uniform vec3 g_Screen;
uniform vec2 g_TexelSize;
uniform vec2 g_TexelSizeHalf;
uniform vec3 u_Expected; // {"material":"expected","default":"1 1 1"}
void main() {
    bool screen = all(lessThan(abs(g_Screen - u_Expected), vec3(0.001)));
    bool texel = all(lessThan(abs(g_TexelSize * u_Expected.xy - vec2(1)), vec2(0.001)));
    bool halfTexel = all(lessThan(abs(g_TexelSizeHalf * 2.0 - g_TexelSize), vec2(0.0000001)));
    float valid = screen && texel && halfTexel ? 1.0 : 0.0;
    float stripe = step(0.5, fract(gl_FragCoord.x * g_TexelSize.x * u_Expected.x * 0.5));
    gl_FragColor = vec4(stripe, valid, 0, 1);
}
''')
        for width, height in [(1280, 720), (640, 360), (853, 479), (479, 853), (1, 1)]:
            material = {"passes": [{"shader": "frame", "textures": ["shared"], "blending": "normal", "constantshadervalues": {"expected": [width, height, width/height]}}]}
            (folder / "material.json").write_text(json.dumps(material))
            shot = folder / f"frame-{width}x{height}.png"
            result = subprocess.run(["cargo", "run", "--offline", "--quiet", "--manifest-path", str(root / "Cargo.toml"), "--example", "view", "--", "--shot-size", f"{width}x{height}", "--shot", str(shot), str(folder)], capture_output=True, text=True)
            if result.returncode:
                raise RuntimeError(result.stderr)
            w, h, channels, pixels = png(shot)
            assert (w, h, channels) == (width, height, 4)
            for y in {0, h//2, h-1}:
                for x in {0, min(w-1, 1), min(w-1, 2), w//2, w-1}:
                    offset = (y*w+x)*channels
                    actual = tuple(pixels[offset:offset+channels])
                    expected = (255*(x%2), 255, 0, 255)
                    assert actual == expected, (width, height, x, y, actual, expected)
            print(f"{width}x{height}: screen dimensions, aspect, texel, half-texel and pixel stripes match")


if __name__ == "__main__":
    main()
