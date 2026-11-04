"""Create ordinary media fixtures for Intel hardware validation (no native projects)."""

import os
from pathlib import Path
import shutil
import subprocess
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
with zipfile.ZipFile(output / "source.ora", "w") as archive:
    archive.writestr("mimetype", "image/openraster")
    archive.writestr("stack.xml", '<image w="160" h="96"><stack><layer name="Top" src="data/top.png" opacity="0.5" visibility="visible" composite-op="svg:src-over" x="0" y="0"/><layer name="Bottom" src="data/bottom.png" opacity="1" visibility="visible" composite-op="svg:src-over" x="0" y="0"/></stack></image>')
    archive.write(output / "source.png", "data/bottom.png")
    archive.write(output / "target.png", "data/top.png")
    archive.write(output / "target.png", "mergedimage.png")
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
