"""Select CUDA 13 dependencies in the image's copy of the uv project."""
from pathlib import Path

manifest = Path("pyproject.toml")
text = manifest.read_text()
for old, new in (
    ('"pycolmap-cuda12>=4.2.1,<5"', '"pycolmap==4.2.1"'),
    ("https://download.pytorch.org/whl/cu126", "https://download.pytorch.org/whl/cu130"),
):
    if text.count(old) != 1:
        raise ValueError(f"Expected exactly one CUDA 12 dependency setting: {old}")
    text = text.replace(old, new)
wheel, = Path("/opt/wheels").glob("pycolmap-*.whl")
text = text.replace("[tool.uv.sources]\n", f'[tool.uv.sources]\npycolmap = {{ path = "{wheel}" }}\n')
manifest.write_text(text)
