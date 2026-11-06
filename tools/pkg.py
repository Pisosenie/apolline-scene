#!/usr/bin/env python3
"""Lists a Wallpaper Engine scene.pkg, or writes one of its files to stdout.

    python3 -I tools/pkg.py scene.pkg            # every file and its size
    python3 -I tools/pkg.py scene.pkg scene.json # one file
"""
import struct
import sys


def entries(path):
    data = open(path, 'rb').read()
    at = 0

    def u32():
        nonlocal at
        value = struct.unpack_from('<I', data, at)[0]
        at += 4
        return value

    tag = u32()  # the version tag's length
    at += tag
    items = []
    for _ in range(u32()):
        length = u32()
        name = data[at:at + length].decode()
        at += length
        offset, size = u32(), u32()
        items.append((name, offset, size))
    return {name: data[at + offset:at + offset + size] for name, offset, size in items}


if __name__ == '__main__':
    files = entries(sys.argv[1])
    if len(sys.argv) == 2:
        for name, data in files.items():
            print(name, len(data))
    else:
        sys.stdout.buffer.write(files[sys.argv[2]])
