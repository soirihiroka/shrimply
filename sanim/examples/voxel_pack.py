"""Generate an original, deterministic block texture pack and runnable survival demo.

Usage: python sanim/examples/voxel_pack.py /tmp/sanim-survival
No downloaded assets or Python dependencies are required.
"""
from pathlib import Path
import random
import shutil
import struct
import sys
import zlib


def png(path, pixels):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    raw = b"".join(b"\0" + bytes(channel for pixel in row for channel in pixel) for row in pixels)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 16, 16, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def texture(name, base, kind):
    rng = random.Random(name)
    pixels = []
    for y in range(16):
        row = []
        for x in range(16):
            value = rng.randrange(-15, 16)
            color = base
            alpha = 255
            if kind == "planks":
                value += -32 if y % 4 == 0 or (x + (y // 4 % 2) * 8) % 16 == 0 else 0
                value += 9 if y % 4 == 1 else 0
            elif kind == "stone":
                value += -30 if y % 8 == 0 or (x + y // 8 * 8) % 8 == 0 else 0
                value += 10 if y % 8 == 1 else 0
            elif kind == "bark":
                value += -24 if x % 4 == 0 else 0
            elif kind == "rings":
                value += -23 if int(max(abs(x - 7.5), abs(y - 7.5))) % 3 == 0 else 4
            elif kind == "grass-side":
                color = (89, 137, 57) if y < 3 + (x * 7 % 3) else (117, 79, 48)
            elif kind == "leaves":
                alpha = 0 if rng.random() < 0.12 else 255
            elif kind == "grass-blade":
                stem = any(abs(x - (center + (15-y)//5 * bend)) <= 1 and y >= top
                           for center, bend, top in [(3, 1, 4), (8, -1, 1), (12, 0, 6)])
                alpha = 255 if stem else 0
                value += (15-y) * 2
            elif kind == "water":
                value += 18 if (x + y * 3) % 9 < 2 else 0
            elif kind == "glass":
                alpha = 255 if x in (0, 15) or y in (0, 15) or x == y else 0
            row.append(tuple(max(0, min(255, c + value)) for c in color) + (alpha,))
        pixels.append(row)
    return pixels


def main():
    if len(sys.argv) != 2:
        raise SystemExit("Usage: python voxel_pack.py OUTPUT_DIRECTORY")
    root = Path(sys.argv[1]).resolve()
    block = root / "assets/minecraft/textures/block"
    block.mkdir(parents=True, exist_ok=True)
    definitions = {
        "grass_block_top": ((91, 143, 59), "noise"),
        "grass_block_side": ((117, 79, 48), "grass-side"),
        "dirt": ((113, 76, 47), "noise"),
        "stone": ((118, 122, 120), "noise"),
        "stone_bricks": ((128, 131, 120), "stone"),
        "cobblestone": ((111, 119, 112), "stone"),
        "oak_planks": ((173, 126, 67), "planks"),
        "oak_log": ((104, 74, 44), "bark"),
        "oak_log_top": ((164, 125, 75), "rings"),
        "dark_oak_planks": ((67, 66, 70), "planks"),
        "oak_leaves": ((64, 116, 53), "leaves"),
        "water_still": ((45, 116, 143), "water"),
        "glass": ((171, 211, 207), "glass"),
        "hay_block_side": ((194, 160, 59), "planks"),
        "sand": ((206, 186, 133), "noise"),
        "short_grass": ((62, 112, 42), "grass-blade"),
    }
    for name, (base, kind) in definitions.items():
        png(block / f"{name}.png", texture(name, base, kind))
    shutil.copyfile(Path(__file__).with_name("voxel_survival.sanim"), root / "survival.sanim")
    shutil.copyfile(Path(__file__).with_name("voxel_structures.sanim"), root / "voxel_structures.sanim")
    valley = Path(__file__).with_name("voxel_valley.sanim")
    if valley.is_file():
        shutil.copyfile(valley, root / "voxel_valley.sanim")
    print(root / "survival.sanim")


if __name__ == "__main__":
    main()
