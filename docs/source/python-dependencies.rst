Python dependency upgrade audit
===============================

Reviewed on 2026-10-23. Python selectors use ``3.14`` in the server, worker,
and documentation environments. Existing version ceilings are preserved where
unchanged; this upgrade introduces no new upper bounds. Lockfiles record the
resolved versions, including the newly tracked Manim worker lockfile.

Compatibility decisions
-----------------------

* Gradio 6.30 requires Pillow below 13, so the server retains Pillow 12.3.
  The Manim worker upgrades to Pillow 13.
* Torchaudio 2.11 is the latest release and explicitly supports later Torch
  releases. It remains paired with Torch 2.14.1 and Torchvision 0.29.1.
* Existing uv overrides/exclusions remain necessary: VGGT requests NumPy below
  2 and the non-headless OpenCV distribution; Qwen requests Transformers 4.57.3
  and unused Sox/ONNX Runtime modules; NeMo requests an older Pynini. Therefore
  a raw ``uv pip check`` reports those pre-existing metadata disagreements.
  Resolution uses Shrimply's overrides, and the supported application paths are
  exercised separately. This is not a claim of compatibility for unused upstream
  features that require the excluded packages.
* Manim advances from ``9d57bcf`` to ``fafa083`` without modifying its source.
  Shrimply adapts texture capture, sampling and array uploads. IR schema 11 rejects
  older packets; the animation cache exists only within the running process.
  Video and sprite rendering matches native Manim, including streamed frames,
  preloading, seeking, looping, transparency and sampling filters.
* Transformers may discard an all-ones attention mask. IndexTTS now derives
  cached audio positions from the KV cache, and cached/uncached logits are
  compared directly during validation.

Source review
-------------

The following inventory covers changed and newly resolved Python packages.
Release notes were reviewed together with intervening releases and linked
migration notes where applicable. Commit history is used where upstream does
not publish versioned release notes. The GTSAM development snapshot is reviewed
against its release-line notes. Platform-only dependencies are resolved by the
worker lockfile; Linux checks do not verify their macOS or Windows runtime.

.. list-table:: Resolved upgrades
   :header-rows: 1
   :widths: 20 16 17 47

   * - Package
     - Previous
     - Selected
     - Review

   * - aiohttp
     - 3.14.3
     - 3.14.5
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/aio-libs/aiohttp/releases/tag/v3.14.5>`__

   * - alabaster
     - 0.7.16
     - 1.0.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/sphinx-doc/alabaster/releases/tag/1.0.0>`__

   * - anyio
     - 4.14.2
     - 4.15.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/agronholm/anyio/releases/tag/4.15.1>`__

   * - av
     - 18.1.0
     - 19.0.2
     - Adapted missing-rate checks and exact fraction conversion for AVRational. `Notes <https://github.com/PyAV-Org/PyAV/releases/tag/v19.0.2>`__

   * - brotli
     - 1.2.0
     - 1.2.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/google/brotli/releases/tag/v1.2.1>`__

   * - certifi
     - 2026.7.22
     - 2026.10.23
     - Reviewed CA removals and restoration of unaffected emSign roots. `Commits <https://github.com/certifi/python-certifi/commits>`__

   * - charset-normalizer
     - 3.5.1
     - 3.5.2
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/jawah/charset_normalizer/releases/tag/3.5.2>`__

   * - click
     - 8.4.2
     - 8.5.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/pallets/click/releases/tag/8.5.0>`__

   * - cloudpickle
     - new
     - 3.1.2
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/cloudpipe/cloudpickle/master/CHANGES.md>`__

   * - contourpy
     - 1.3.3
     - 1.4.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/contourpy/contourpy/releases/tag/v1.4.0>`__

   * - cuda-bindings
     - 12.9.7
     - 12.9.10
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://nvidia.github.io/cuda-python/cuda-bindings/latest/release.html>`__

   * - cuda-pathfinder
     - 1.7.0
     - 1.8.4
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://nvidia.github.io/cuda-python/cuda-pathfinder/latest/release/1.8.4-notes.html>`__

   * - cyclopts
     - 4.23.2
     - 5.3.0
     - Reviewed v5 parsing and exit-code changes; existing worker CLI remains compatible. `Notes <https://github.com/BrianPugh/cyclopts/releases/tag/v5.3.0>`__

   * - diffusers
     - 0.40.0.dev0
     - 0.42.0
     - Released MiniMax H3/Wan APIs replace the development commit; dtype API reviewed. `Notes <https://github.com/huggingface/diffusers/releases/tag/v0.42.0>`__

   * - docutils
     - 0.19
     - 0.23
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://docutils.sourceforge.io/HISTORY.html>`__

   * - executing
     - 2.2.1
     - 2.3.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Commits <https://github.com/alexmojaki/executing/compare/v2.2.1...v2.3.0>`__

   * - fastapi
     - 0.141.1
     - 0.162.0
     - Reviewed intervening breaking changes; Shrimply uses explicit raw-body routes, not form defaults. `Notes <https://github.com/fastapi/fastapi/releases/tag/0.162.0>`__

   * - filelock
     - 3.32.4
     - 4.2.4
     - Reviewed v4 ownership/context changes; model-download dependencies import successfully. `Notes <https://github.com/tox-dev/py-filelock/releases/tag/4.2.4>`__

   * - fonttools
     - 4.63.0
     - 4.67.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/fonttools/fonttools/releases/tag/4.67.0>`__

   * - fsspec
     - 2026.7.0
     - 2026.9.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://filesystem-spec.readthedocs.io/en/latest/changelog.html>`__

   * - furo
     - 2025.9.25
     - 2025.12.19
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/pradyunsg/furo/releases/tag/2025.12.19>`__

   * - glfw
     - 2.10.2
     - 2.11.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/FlorianRhiem/pyGLFW/releases/tag/v2.11.0>`__

   * - gradio
     - 6.26.0
     - 6.30.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/gradio-app/gradio/releases/tag/gradio@6.30.0>`__

   * - gradio-client
     - 2.6.1
     - 2.7.2
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/gradio-app/gradio/main/CHANGELOG.md>`__

   * - gtsam-develop
     - 4.3a2.dev202608252015
     - 4.3.2.dev202610230405
     - Reviewed GTSAM 4.3 removals; retained existing development distribution. `Notes <https://github.com/borglab/gtsam/releases/tag/4.3.0>`__

   * - hf-xet
     - 1.6.0
     - 1.7.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/huggingface/xet-core/releases/tag/v1.7.0>`__

   * - httpcore2
     - new
     - 2.16.0
     - Transport paired with httpx2; no direct Shrimply calls. `Notes <https://raw.githubusercontent.com/pydantic/httpx2/main/src/httpcore2/CHANGELOG.md>`__

   * - httpx2
     - new
     - 2.16.0
     - Hub HTTP transport; server requests verified with the installed stack. `Notes <https://raw.githubusercontent.com/pydantic/httpx2/main/src/httpx2/CHANGELOG.md>`__

   * - huggingface-hub
     - 1.28.0
     - 2.3.0
     - HTTP stack changes to httpx2; download/progress adapters and imports checked. `Notes <https://github.com/huggingface/huggingface_hub/releases/tag/v2.3.0>`__

   * - idna
     - 3.19
     - 3.20
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/kjd/idna/releases/tag/v3.20>`__

   * - importlib-metadata
     - 9.0.0
     - 10.1.0
     - Python 3.10 support removed; supported by the Python 3.14 runtime. `Notes <https://raw.githubusercontent.com/python/importlib_metadata/main/NEWS.rst>`__

   * - inflect
     - 7.5.0
     - 7.6.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/jaraco/inflect/main/NEWS.rst>`__

   * - iniconfig
     - 2.3.0
     - 2.3.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/pytest-dev/iniconfig/releases/tag/v2.3.1>`__

   * - ipython
     - 9.16.1
     - 9.17.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Commits <https://github.com/ipython/ipython/compare/9.16.1...9.17.1>`__

   * - jaraco-functools
     - new
     - 4.6.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/jaraco/jaraco.functools/main/NEWS.rst>`__

   * - jedi
     - 0.20.0
     - 0.20.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/davidhalter/jedi/master/CHANGELOG.rst>`__

   * - joblib
     - 1.5.3
     - 1.6.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/joblib/joblib/main/CHANGES.rst>`__

   * - kiwisolver
     - 1.5.0
     - 1.5.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/nucleic/kiwi/releases/tag/1.5.1>`__

   * - lazy-loader
     - 0.5
     - 0.6
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/scientific-python/lazy-loader/releases/tag/v0.6>`__

   * - llvmlite
     - 0.49.0
     - 0.50.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/numba/llvmlite/releases/tag/v0.50.0>`__

   * - manimpango
     - 0.6.1
     - 0.7.0
     - Python 3.10+ and glyph rounding changes; native visual comparisons passed. `Notes <https://github.com/ManimCommunity/ManimPango/releases/tag/v0.7.0>`__

   * - markupsafe
     - 3.0.3
     - 3.0.4
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/pallets/markupsafe/releases/tag/3.0.4>`__

   * - matplotlib
     - 3.11.1
     - 3.11.2
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/matplotlib/matplotlib/releases/tag/v3.11.2>`__

   * - more-itertools
     - 11.1.0
     - 11.2.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/more-itertools/more-itertools/releases/tag/v11.2.1>`__

   * - msgpack
     - 1.2.1
     - 1.2.3
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/msgpack/msgpack-python/releases/tag/v1.2.3>`__

   * - msgspec
     - 0.21.1
     - 0.22.0
     - gc=False/weakref restriction does not apply to Shrimply; MessagePack endpoints checked. `Notes <https://github.com/msgspec/msgspec/releases/tag/0.22.0>`__

   * - multidict
     - 6.7.1
     - 7.1.0
     - Reviewed unique-key iteration and istr subclass restriction; no direct Shrimply calls. `Notes <https://github.com/aio-libs/multidict/releases/tag/v7.1.0>`__

   * - narwhals
     - 2.25.0
     - 2.27.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/narwhals-dev/narwhals/releases/tag/v2.27.1>`__

   * - networkx
     - 3.6.1
     - 3.7
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://networkx.org/documentation/stable/release/release_3.7.html>`__

   * - numba
     - 0.67.0
     - 0.68.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/numba/numba/releases/tag/0.68.0>`__

   * - numpy
     - 2.5.2
     - 2.5.4
     - Reviewed non-finite linalg input checks and patch fixes; finite media/GPU paths checked. `Notes <https://github.com/numpy/numpy/releases/tag/v2.5.4>`__

   * - nvidia-nvjitlink-cu12
     - 12.6.85
     - 12.9.86
     - CUDA 12.9 component within the CUDA 12 family; GPU compilation checked. `Notes <https://docs.nvidia.com/cuda/archive/12.9.1/cuda-toolkit-release-notes/index.html>`__

   * - onnx
     - 1.22.0
     - 1.23.2
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/onnx/onnx/releases/tag/v1.23.2>`__

   * - opentelemetry-api
     - new
     - 1.45.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/open-telemetry/opentelemetry-python/releases/tag/v1.45.1>`__

   * - orjson
     - 3.12.0
     - 3.13.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/ijl/orjson/releases/tag/3.13.0>`__

   * - pandas
     - 3.0.5
     - 3.1.0
     - Reviewed datetime resolution and array frequency changes; no direct Shrimply calls. `Notes <https://pandas.pydata.org/docs/whatsnew/v3.1.0.html>`__

   * - peft
     - 0.20.0
     - 0.21.2
     - Includes compatibility fixes for Transformers >=5.18 and TorchAO 0.18. `Notes <https://github.com/huggingface/peft/releases/tag/v0.21.2>`__

   * - pillow
     - 12.3.0
     - 13.0.0
     - Worker uses Pillow 13; server retains Pillow 12 because Gradio requires it. `Notes <https://github.com/python-pillow/Pillow/releases/tag/13.0.0>`__

   * - platformdirs
     - 4.11.4
     - 4.13.5
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/tox-dev/platformdirs/releases/tag/4.13.5>`__

   * - pluggy
     - 1.6.0
     - 1.7.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://pluggy.readthedocs.io/en/latest/changelog.html>`__

   * - propcache
     - 0.5.2
     - 0.5.4
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/aio-libs/propcache/releases/tag/v0.5.4>`__

   * - protobuf
     - 7.36.0
     - 7.36.2
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/protocolbuffers/protobuf/releases/tag/v36.2>`__

   * - pure-eval
     - 0.2.3
     - 0.2.4
     - No Shrimply-facing migration identified in the reviewed release notes. `Commits <https://github.com/alexmojaki/pure_eval/compare/v0.2.3...v0.2.4>`__

   * - pycolmap-cuda12
     - 4.1.1
     - 4.2.1
     - Reviewed 4.2 changes and hash-map ABI fix; CUDA support checked. `Notes <https://github.com/colmap/colmap/releases/tag/4.2.1>`__

   * - pycparser
     - 3.0
     - 3.12
     - No Shrimply-facing migration identified in the reviewed release notes. `Commits <https://github.com/eliben/pycparser/commits>`__

   * - pydantic
     - 2.13.4
     - 2.14.1
     - Reviewed 2.14 schema/validation changes; protocol models and endpoints checked. `Notes <https://raw.githubusercontent.com/pydantic/pydantic/main/HISTORY.md>`__

   * - pydantic-core
     - 2.46.4
     - 2.50.1
     - Updated with its matching Pydantic release; strict protocol validation checked. `Notes <https://raw.githubusercontent.com/pydantic/pydantic/main/HISTORY.md>`__

   * - pyparsing
     - 3.3.2
     - 3.3.3
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/pyparsing/pyparsing/releases/tag/3.3.3>`__

   * - pyrefly
     - 1.2.0
     - 1.4.0
     - Stricter inference exposed nullable channels and decorated method signatures; fixed call sites. `Notes <https://github.com/facebook/pyrefly/releases/tag/1.4.0>`__

   * - python-dateutil
     - 2.9.0.post0
     - 3.0.0
     - Reviewed timezone API changes; no direct Shrimply calls. `Notes <https://github.com/dateutil/dateutil/releases/tag/3.0.0>`__

   * - pytorch-lightning
     - 2.6.5
     - 2.6.6
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/Lightning-AI/pytorch-lightning/releases/tag/2.6.6>`__

   * - pytz
     - 2026.3.post1
     - 2026.5
     - No Shrimply-facing migration identified in the reviewed release notes. `Commits <https://github.com/stub42/pytz/commits>`__

   * - regex
     - 2026.7.19
     - 2026.9.29
     - No Shrimply-facing migration identified in the reviewed release notes. `Commits <https://github.com/mrabarnett/mrab-regex/compare/2026.7.19...2026.9.29>`__

   * - rich-rst
     - 2.1.0
     - 2.2.0
     - Includes directive file-access restrictions; worker CLI help checked. `Notes <https://github.com/wasi-master/rich-rst/releases/tag/v2.2.0>`__

   * - roman-numerals
     - new
     - 4.1.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/AA-Turner/roman-numerals/master/CHANGES.rst>`__

   * - rubicon-objc
     - 0.5.6
     - 0.5.7
     - Python 3.11 minimum and platform processor requirements; macOS runtime needs platform validation. `Notes <https://github.com/beeware/rubicon-objc/releases/tag/v0.5.7>`__

   * - scikit-learn
     - 1.9.0
     - 1.9.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://scikit-learn.org/stable/whats_new/v1.9.html>`__

   * - setuptools
     - 84.0.0
     - 84.2.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://setuptools.pypa.io/en/stable/history.html>`__

   * - soupsieve
     - 2.9.2
     - 3.0.4
     - Reviewed v3 Python minimum and selector changes; documentation builds passed. `Notes <https://github.com/facelessuser/soupsieve/releases/tag/3.0.4>`__

   * - sphinx
     - 6.2.0
     - 9.2.0
     - Reviewed 7/8/9 migrations; upgraded theme/extensions and warning-as-error build passed. `Notes <https://github.com/sphinx-doc/sphinx/releases/tag/v9.2.0>`__

   * - sphinx-tabs
     - 3.4.7
     - 3.5.0
     - Raises minimum Sphinx to 7; documentation builds on Sphinx 9. `Notes <https://github.com/executablebooks/sphinx-tabs/releases/tag/v3.5.0>`__

   * - sphinxcontrib-jsmath
     - 1.0.1
     - 2.0.0
     - Supports current Sphinx math context and Python 3.14. `Notes <https://raw.githubusercontent.com/sphinx-doc/sphinxcontrib-jsmath/master/CHANGES.rst>`__

   * - starlette
     - 1.6.0
     - 1.9.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://starlette.dev/release-notes/>`__

   * - threadpoolctl
     - 3.6.0
     - 3.7.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/joblib/threadpoolctl/master/CHANGES.md>`__

   * - tokenizers
     - 0.22.2
     - 0.23.3
     - Reviewed normalization change and overflowing-token patch; paired with Transformers. `Notes <https://github.com/huggingface/tokenizers/releases/tag/v0.23.3>`__

   * - torch
     - 2.13.0+cu126
     - 2.14.1+cu126
     - Reviewed 2.14 changes; GPU execution, compilation and INT8 quantization checked. `Notes <https://github.com/pytorch/pytorch/releases/tag/v2.14.1>`__

   * - torch-einops-utils
     - 0.1.21
     - 0.1.42
     - No Shrimply-facing migration identified in the reviewed release notes. `Commits <https://github.com/lucidrains/torch-einops-utils/commits>`__

   * - torchvision
     - 0.28.0+cu126
     - 0.29.1+cu126
     - Matching 0.29.1 release; image codec deprecations do not affect Shrimply media decoding. `Notes <https://github.com/pytorch/vision/releases/tag/v0.29.1>`__

   * - tqdm
     - 4.70.0
     - 4.70.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/tqdm/tqdm/releases/tag/v4.70.1>`__

   * - transformers
     - 5.15.1
     - 5.20.2
     - Adapted IndexTTS cached token positions when generation drops an all-ones mask. `Notes <https://github.com/huggingface/transformers/releases/tag/v5.20.2>`__

   * - trimesh
     - 5.0.0
     - 5.1.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/mikedh/trimesh/releases/tag/5.1.1>`__

   * - triton
     - 3.7.1
     - 3.8.0
     - Reviewed 3.8 compiler changes; compiled GPU operation checked. `Notes <https://github.com/triton-lang/triton/releases/tag/v3.8.0>`__

   * - truststore
     - new
     - 0.10.5
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/sethmlarson/truststore/releases/tag/v0.10.5>`__

   * - typer
     - 0.27.1
     - 0.27.3
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/fastapi/typer/releases/tag/0.27.3>`__

   * - types-tqdm
     - 4.70.0.20260805
     - 4.70.0.20260906
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/typeshed-internal/stub_uploader/main/data/changelogs/tqdm.md>`__

   * - tzdata
     - new
     - 2026.5
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/python/tzdata/releases/tag/2026.5>`__

   * - urllib3
     - 2.7.0
     - 2.8.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/urllib3/urllib3/releases/tag/2.8.0>`__

   * - uv
     - 0.12.6
     - 0.13.5
     - Reviewed 0.13 changes and managed interpreter/certificate fixes; used in Docker builds. `Notes <https://github.com/astral-sh/uv/releases/tag/0.13.5>`__

   * - uvicorn
     - 0.52.4
     - 0.54.0
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://uvicorn.dev/release-notes>`__

   * - validators
     - 0.35.0
     - 0.36.0
     - Stricter checks and Python 3.11 minimum; worker CLI remains compatible. `Notes <https://github.com/python-validators/validators/releases/tag/0.36.0>`__

   * - wcwidth
     - 0.8.2
     - 0.9.3
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/jquast/wcwidth/releases/tag/0.9.3>`__

   * - wrapt
     - 2.3.0
     - 2.5.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/GrahamDumpleton/wrapt/releases/tag/2.5.1>`__

   * - yarl
     - 1.24.5
     - 1.25.1
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://github.com/aio-libs/yarl/releases/tag/v1.25.1>`__

   * - zipp
     - 4.1.0
     - 4.1.2
     - No Shrimply-facing migration identified in the reviewed release notes. `Notes <https://raw.githubusercontent.com/jaraco/zipp/main/NEWS.rst>`__

Manim revision notes: `VideoMobject and Sprite
<https://github.com/3b1b/manim/commit/fafa083a4fb274bba9cabde0b6e2f50ba6da0622>`__.

Container variants
------------------

CUDA 12.6 remains the default. CI also builds CUDA 13.0, using copies of the
manifest and lockfile transformed inside the Docker build. The CUDA 13 variant
builds PyCOLMAP 4.2.1 from pinned upstream source, packages its native dependencies,
and installs it instead of the CUDA 12 wheel. Runtime dependencies are installed
at image build time; application startup performs no dependency resolution.

The native wheel stage precedes application source copies. CI exports intermediate
layers with ``mode=max`` in separate GHCR cache images for each CUDA variant, so source-only changes
reuse the wheel and dependency layers. A source revision, toolkit or build-setting
change invalidates the corresponding native build layer.

See `COLMAP 4.2.1 notes
<https://github.com/colmap/colmap/releases/tag/4.2.1>`__ and `CUDA 13.0 Update 2 notes
<https://docs.nvidia.com/cuda/archive/13.0.2/cuda-toolkit-release-notes/index.html>`__.
CUDA 13 requires a compatible NVIDIA driver and Turing or newer GPUs; use the
CUDA 12.6 variant for older supported hardware.

The CUDA 13 runtime selected by Torch uses `CUDA 13.0 Update 3
<https://docs.nvidia.com/cuda/archive/13.0.3/cuda-toolkit-release-notes/index.html>`__,
which includes a cuBLAS correctness fix. The wheel builder uses the compatible
CUDA 13.0 Update 2 development toolkit.

Build tooling reviewed for the new native wheel stage includes
`auditwheel 6.8.2 <https://github.com/pypa/auditwheel/releases/tag/6.8.2>`__
(deterministic SBOM fix),
`scikit-build-core 1.1.1
<https://github.com/scikit-build/scikit-build-core/releases/tag/v1.1.1>`__
(wheel performance and interpreter discovery fixes), and upstream's pinned
`pybind11 3.0.4 <https://github.com/pybind/pybind11/releases/tag/v3.0.4>`__
(Python 3.13+ traversal and C++ ownership fixes).

The wheel repair stage pins `patchelf 0.19.2
<https://github.com/NixOS/patchelf/blob/0.19.2/ChangeLog.md>`__ and verifies its
release archive checksum. Its segment-alignment and binary-corruption fixes
prevent the invalid OpenBLAS mappings produced by Ubuntu's patchelf 0.18.0.
OpenGL dispatch, GLX, and OpenGL libraries remain supplied together by the runtime
image, alongside XRender, to avoid mixing bundled and system graphics libraries.
