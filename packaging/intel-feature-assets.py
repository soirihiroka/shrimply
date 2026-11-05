"""Create ordinary media fixtures for Intel hardware validation (no native projects)."""

import os
from pathlib import Path
import shutil
import subprocess
import struct
import sys
import zipfile

output = Path(sys.argv[1]).resolve()
objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 160 96] /Contents 4 0 R >>",
]
content = b"0.9 0.25 0.1 rg 20 20 120 56 re f\n"
objects.append(b"<< /Length " + str(len(content)).encode() + b" >>\nstream\n" + content + b"endstream")
pdf = bytearray(b"%PDF-1.4\n")
offsets = [0]
for index, value in enumerate(objects, 1):
    offsets.append(len(pdf))
    pdf.extend(f"{index} 0 obj\n".encode() + value + b"\nendobj\n")
xref = len(pdf)
pdf.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
for offset in offsets[1:]:
    pdf.extend(f"{offset:010d} 00000 n \n".encode())
pdf.extend(f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode())
(output / "source.pdf").write_bytes(pdf)
def rgba(path):
    return subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", str(path), "-f", "rawvideo", "-pix_fmt", "rgba", "-"], check=True, stdout=subprocess.PIPE).stdout

source = rgba(output / "source.png")
target = rgba(output / "target.png")
records = bytearray()
channels = bytearray()
width, height = 160, 96
for name, opacity, pixels in [(b"Top", 128, target), (b"Bottom", 255, source)]:
    records.extend(struct.pack(">4iH", 0, 0, height, width, 4))
    for channel in [0, 1, 2, -1]:
        records.extend(struct.pack(">hI", channel, width * height + 2))
        channels.extend(b"\0\0" + pixels[(channel if channel >= 0 else 3)::4])
    records.extend(b"8BIMnorm" + bytes([opacity, 0, 0, 0]))
    pascal = bytes([len(name)]) + name
    pascal += b"\0" * (-len(pascal) % 4)
    extra = b"\0" * 8 + pascal
    records.extend(struct.pack(">I", len(extra)) + extra)
layer_info = struct.pack(">h", 2) + records + channels
layer_info += b"\0" * (len(layer_info) % 2)
layer_section = struct.pack(">I", len(layer_info)) + layer_info + b"\0" * 4
header = b"8BPS" + struct.pack(">H6sHIIHH", 1, b"\0" * 6, 4, height, width, 8, 3)
composite = b"\0\0" + b"".join(source[channel::4] for channel in range(4))
(output / "source.psd").write_bytes(header + b"\0" * 8 + struct.pack(">I", len(layer_section)) + layer_section + composite)
with zipfile.ZipFile(output / "source.kra", "w") as archive:
    archive.writestr("mimetype", "application/x-krita")
    archive.writestr("maindoc.xml", '<DOC><IMAGE name="Validation" width="160" height="96" colorspacename="RGBA"><layers><layer name="Top" nodetype="shapelayer" filename="layer1" opacity="128" visible="1" compositeop="normal"/><layer name="Bottom" nodetype="shapelayer" filename="layer0" opacity="255" visible="1" compositeop="normal"/></layers></IMAGE></DOC>')
    archive.write(output / "source.svg", "Validation/layers/layer0.shapelayer/content.svg")
    archive.writestr("Validation/layers/layer1.shapelayer/content.svg", '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="96"><circle cx="80" cy="48" r="32" fill="#168edd"/></svg>')
subprocess.run([
    "ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
    "testsrc2=size=160x96:rate=30", "-frames:v", "6", "-c:v", "libx264",
    "-pix_fmt", "yuv420p", str(output / "source.mp4"),
], check=True)
blender = os.environ.get("BLENDER_BINARY") or shutil.which("blender")
if not blender:
    raise RuntimeError("Blender is required for complete feature validation; set BLENDER_BINARY")
subprocess.run([
    blender, "-b", "--factory-startup", "--python-expr",
    "import bpy; "
    "bpy.context.scene.render.engine='BLENDER_EEVEE'; "
    "bpy.context.scene.render.resolution_x=160; "
    "bpy.context.scene.render.resolution_y=96; "
    "bpy.context.scene.render.resolution_percentage=100; "
    "bpy.context.scene.render.film_transparent=True; "
    "bpy.context.scene.render.fps=30; "
    "bpy.context.scene.frame_end=6; "
    f"bpy.ops.wm.save_as_mainfile(filepath={str(output / 'source.blend')!r})",
], check=True)
