# Deterministic GL snapshots of active, inactive and optimized-away time uniforms.
import json
from pathlib import Path
import runpy
import struct
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parents[1]
    png = runpy.run_path(str(root / "tools/check-material-channels.py"))["png"]
    with tempfile.TemporaryDirectory(prefix="apolline-time-") as temporary:
        folder = Path(temporary)
        (folder / "materials").mkdir()
        (folder / "shaders").mkdir()
        texture = b"TEXV0005\0TEXI0001\0" + struct.pack("<7I", 0, 0, 1, 1, 1, 1, 0)
        texture += b"TEXB0002\0" + struct.pack("<7I", 1, 1, 1, 1, 0, 0, 4) + bytes([255] * 4)
        (folder / "materials/shared.tex").write_bytes(texture)
        (folder / "project.json").write_text("{}")
        (folder / "model.json").write_text('{"material":"material.json","fullscreen":true}')
        (folder / "material.json").write_text('{"passes":[{"shader":"time","textures":["shared"],"blending":"normal"}]}')
        (folder / "scene.json").write_text('{"general":{"orthogonalprojection":{"width":1280,"height":720}},"objects":[{"image":"model.json"}]}')

        def shot(name, source, time, animated):
            (folder / "shaders/time.frag").write_text(source)
            image = folder / f"{name}.png"
            result = subprocess.run(["cargo", "run", "--offline", "--quiet", "--manifest-path", str(root / "Cargo.toml"), "--example", "view", "--", "--stats", "--shot-size", "64x64", "--time", str(time), "--shot", str(image), str(folder)], capture_output=True, text=True)
            if result.returncode:
                raise RuntimeError(result.stderr)
            assert f"animated={str(animated).lower()}" in result.stderr, result.stderr
            return png(image)

        source = "uniform float g_Time;\nuniform float g_Frametime;\nvoid main() { gl_FragColor = vec4(fract(g_Time), g_Frametime, 0.25, 1); }"
        frames = []
        for time in (0, 0.25, 0.5, 0.75):
            frame = shot(str(time), source, time, True)
            actual = tuple(frame[3][:4])
            expected = (round(time*255), 0, 64, 255)
            assert all(abs(a-b) <= 1 for a, b in zip(actual, expected)), (time, actual, expected)
            frames.append(frame)
        assert frames[2] == shot("same-time", source, 0.5, True)
        unused = "uniform float g_Time;\nvoid main() { gl_FragColor = vec4(0.25, 0, 0.25, 1); }"
        inactive = "#if 0\nuniform float g_Time;\n#endif\nvoid main() { gl_FragColor = vec4(0.25, 0, 0.25, 1); }"
        assert shot("unused", unused, 10, False) == shot("inactive", inactive, 20, False)
        print("Time changes GPU colors at 0/0.25/0.5/0.75s; fixed time is identical; inactive and optimized-away clocks remain static.")


if __name__ == "__main__":
    main()
