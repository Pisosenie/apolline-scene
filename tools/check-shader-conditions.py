# Differential GPU check of the supported conditional subset against a C preprocessor.
import argparse
import json
import os
from pathlib import Path
import random
import runpy
import shlex
import struct
import subprocess
import tempfile


def cases():
    expressions = [
        "MODE", "!MODE", "MODE == 2", "MODE < 1", "MODE >= 0",
        "defined GLSL && !defined HLSL", "defined(GUARD)", "defined(FUNCTION)",
        "A * 3 == 7", "(A) * 3 == 9", "B & 8", "B ^ 3", "UNDEFINED",
        "1 || 1 / 0", "0 && 1 / 0", "MODE && 3 / MODE", "!MODE || 3 / MODE",
        "2 & 1 == 0", "4 | 1 && 0", "1 << 2 + 1", "0x10 >> 2 == 4",
        "-3 / 2 == -1", "-3 % 2 == -1", "~MODE & 7", "defined(ABSENT)",
    ]
    generator = random.Random(112358)
    atoms = ["MODE", "A", "B", "0", "1", "2", "3", "-2"]
    operators = ["+", "-", "*", "&", "|", "^", "==", "!=", "<", ">=", "&&", "||"]
    while len(expressions) < 64:
        a, b, c = generator.choices(atoms, k=3)
        first, second = generator.choices(operators, k=2)
        expressions.append(f"{a} {first} {b} {second} {c}")
    return [(expression, [-2, 0, 1, 2, 7][index % 5]) for index, expression in enumerate(expressions)]


DEFINES = "#define A 1 + 2\n#define B (3 | 8)\n#define GUARD\n#define FUNCTION(x) (x)\n"


def reference(items):
    source = "#define GLSL 1\n" + DEFINES
    for index, (expression, mode) in enumerate(items):
        source += f"#define MODE {mode}\n#if {expression}\ncase_{index}_true\n#else\ncase_{index}_false\n#endif\n#undef MODE\n"
    command = shlex.split(os.environ.get("CC", "cc")) + ["-E", "-P", "-x", "c", "-"]
    result = subprocess.run(command, input=source, capture_output=True, text=True, check=True)
    values = {}
    for line in result.stdout.splitlines():
        if line.startswith("case_"):
            _, index, selected = line.split("_")
            values[int(index)] = selected == "true"
    assert len(values) == len(items), result.stdout
    return values


def fixture(folder, items):
    for name in ("models", "materials", "shaders"):
        (folder / name).mkdir()
    texture = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", 0, 0, 1, 1, 1, 1, 0)
    texture += b"TEXB0002\0" + struct.pack("<7I", 1, 1, 1, 1, 0, 0, 4) + bytes([255] * 4)
    (folder / "materials/shared.tex").write_bytes(texture)
    (folder / "project.json").write_text("{}")
    (folder / "shaders/guard.h").write_text("#ifndef LOCAL_HEADER\n#define LOCAL_HEADER\n#if 0\nuniform float g_Time;\n#endif\n#endif\n")
    objects = []
    for index, (expression, mode) in enumerate(items):
        x, y = index % 8, index // 8
        objects.append({"image": f"models/{index}.json", "origin": f"{x * 160 + 80} {720 - y * 90 - 45} 0", "size": "160 90"})
        (folder / f"models/{index}.json").write_text(json.dumps({"material": f"materials/{index}.json"}))
        (folder / f"materials/{index}.json").write_text(json.dumps({"passes": [{"shader": str(index), "textures": ["shared"], "combos": {"MODE": mode}, "blending": "normal"}]}))
        source = '// [COMBO] {"combo":"MODE","default":0}\n' + DEFINES
        source += '#include "guard.h"\n#include "guard.h"\n'
        source += f"#if {expression}\nvoid main() {{ gl_FragColor = vec4(1,0,0,1); }}\n#else\nvoid main() {{ gl_FragColor = vec4(0,1,0,1); }}\n#endif\n"
        source += "#if 0\ninvalid active syntax;\nuniform float g_Time;\n#elif 0\nuniform sampler2D g_Texture7;\n#endif\n"
        (folder / f"shaders/{index}.frag").write_text(source)
    (folder / "scene.json").write_text(json.dumps({"general": {"orthogonalprojection": {"width": 1280, "height": 720}}, "objects": objects}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shot", type=Path, help="retain the GPU test image at this path")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    png = runpy.run_path(str(root / "tools/check-material-channels.py"))["png"]
    items = cases()
    expected = reference(items)
    with tempfile.TemporaryDirectory(prefix="apolline-conditions-") as temporary:
        folder = Path(temporary)
        fixture(folder, items)
        shot = args.shot.resolve() if args.shot else folder / "frame.png"
        result = subprocess.run(["cargo", "run", "--offline", "--quiet", "--manifest-path", str(root / "Cargo.toml"), "--example", "view", "--", "--stats", "--shot", str(shot), str(folder)], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(result.stderr)
        assert "1 texture uploads, 4 logical texture bytes" in result.stderr, result.stderr
        width, height, channels, pixels = png(shot)
        assert (width, height, channels) == (1280, 720, 4)
        for index, (expression, mode) in enumerate(items):
            x, y = index % 8 * 160 + 80, index // 8 * 90 + 45
            at = (y * width + x) * channels
            actual = tuple(pixels[at:at + channels])
            color = (255, 0, 0, 255) if expected[index] else (0, 255, 0, 255)
            assert actual == color, (index, expression, mode, actual, color)
    print(f"{len(items)} conditional variants match the C preprocessor and GPU pixels; guarded headers and inactive dynamic uniforms pass.")


if __name__ == "__main__":
    main()
