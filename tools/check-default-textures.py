# GPU regression for shader default resources, overrides and lookup precedence.
import json
from pathlib import Path
import runpy
import struct
import subprocess
import tempfile


def texture(fmt, width, pixels):
    return (b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", fmt, 0, width, 1, width, 1, 0)
            + b"TEXB0002\0" + struct.pack("<7I", 1, 1, width, 1, 0, 0, len(pixels)) + pixels)


def package(path, name, data):
    def string(text):
        raw = text.encode()
        return struct.pack("<I", len(raw)) + raw
    path.write_bytes(string("PKGV0021") + struct.pack("<I", 1) + string(name) + struct.pack("<II", 0, len(data)) + data)


def main():
    root = Path(__file__).resolve().parents[1]
    png = runpy.run_path(str(root / "tools/check-material-channels.py"))["png"]
    with tempfile.TemporaryDirectory(prefix="apolline-defaults-") as temporary:
        folder, assets = Path(temporary) / "project", Path(temporary) / "assets"
        for directory in (folder / "materials", folder / "shaders", assets / "materials"):
            directory.mkdir(parents=True, exist_ok=True)
        base = (180, 60, 30)
        (assets / "materials/base.tex").write_bytes(texture(0, 1, bytes(base + (255,))))
        (assets / "materials/mask.tex").write_bytes(texture(9, 2, bytes([0, 255])))
        (folder / "project.json").write_text("{}")
        (folder / "model.json").write_text('{"material":"material.json","fullscreen":true}')
        (folder / "scene.json").write_text('{"general":{"orthogonalprojection":{"width":64,"height":32},"clearcolor":"0.05 0.05 0.1"},"objects":[{"image":"model.json"}]}')
        (folder / "shaders/main.frag").write_text('#include "textures.h"\n#include "textures.h"\nvoid main() { vec4 pixel = texSample2D(g_Texture0, v_TexCoord);\n#if MASK\npixel.a *= texSample2D(g_Texture7, v_TexCoord).r * step(1.0, g_Texture7Resolution.x);\n#endif\ngl_FragColor = pixel; }')

        def header(mask="mask"):
            (folder / "shaders/textures.h").write_text(f'''#ifndef TEXTURES_HEADER
#define TEXTURES_HEADER
varying vec2 v_TexCoord;
uniform sampler2D g_Texture0; // {{"default":"base","hidden":true}}
uniform sampler2D g_Texture7; // {{"default":"{mask}","combo":"MASK","mode":"opacitymask"}}
uniform vec4 g_Texture7Resolution;
#endif
''')

        def shot(label, names, right, *, full=False, uploads=2, logical=6, error=None, alpha=1):
            material = {"shader": "main", "blending": "translucent"}
            if names is not None:
                material["textures"] = names
            (folder / "material.json").write_text(json.dumps({"passes": [material]}))
            frame = folder / f"{label}.png"
            result = subprocess.run(["cargo", "run", "--offline", "--quiet", "--manifest-path", str(root / "Cargo.toml"), "--example", "view", "--", "--stats", "--shot-size", "64x32", "--shot", str(frame), str(folder), str(assets)], capture_output=True, text=True)
            if error:
                assert result.returncode != 0 and error in result.stderr, result.stderr
                assert not frame.exists(), label
                return
            if result.returncode:
                raise RuntimeError(result.stderr)
            assert f"{uploads} texture uploads, {logical} logical texture bytes" in result.stderr, result.stderr
            width, height, channels, pixels = png(frame)
            assert (width, height, channels) == (64, 32, 4)
            for x in (2, 61):
                expected = right if (x == 61 or full) else (13, 13, 26)
                expected = tuple(round(a*(1-alpha)+b*alpha) for a,b in zip((13, 13, 26), expected)) + (255,)
                at = (16*width+x)*channels
                actual = tuple(pixels[at:at+channels])
                assert all(abs(a-b) <= 2 for a,b in zip(actual, expected)), (label, x, actual, expected)
            print(f"{label}: defaults/overrides, pixels and unique uploads match")

        header()
        shot("assets", None, base)
        shot("empty-list", [], base)
        shot("null-primary", [None], base)
        override = (30, 180, 90)
        (folder / "materials/override.tex").write_bytes(texture(0, 1, bytes(override + (255,))))
        shot("explicit-primary", ["override"], override)
        (folder / "materials/solid.tex").write_bytes(texture(9, 1, bytes([255])))
        shot("explicit-secondary", [None] * 7 + ["solid"], base, full=True, logical=5)
        local = (90, 30, 180)
        (folder / "materials/base.tex").write_bytes(texture(0, 1, bytes(local + (255,))))
        shot("folder-before-assets", None, local)
        packed = (10, 120, 230)
        package(folder / "scene.pkg", "materials/base.tex", texture(0, 1, bytes(packed + (255,))))
        shot("package-before-folder", None, packed)
        (folder / "scene.pkg").unlink()
        (folder / "materials/base.tex").unlink()
        header("base")
        shot("shared-default", None, base, full=True, uploads=1, logical=4, alpha=base[0]/255)
        header()
        shot("missing-explicit", ["missing"], None, error="missing.tex")
        (assets / "materials/mask.tex").unlink()
        shot("missing-default", None, None, error="mask.tex")
        print("Missing named/default files fail without partial screenshots; defaults never replace a broken explicit override.")


if __name__ == "__main__":
    main()
