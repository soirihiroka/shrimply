# syntax=docker/dockerfile:1
ARG CUDA_VERSION=12.6

FROM nvidia/cuda:13.0.2-devel-ubuntu24.04 AS pycolmap-cuda13
COPY --from=ghcr.io/astral-sh/uv:0.13.5 /uv /usr/local/bin/uv
ENV DEBIAN_FRONTEND=noninteractive UV_PYTHON=3.14 UV_LINK_MODE=copy
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential ca-certificates cmake git ninja-build patchelf \
    libboost-program-options-dev libboost-graph-dev libboost-system-dev \
    libeigen3-dev libopenimageio-dev openimageio-tools libmetis-dev libgoogle-glog-dev \
    libgtest-dev libgmock-dev libsqlite3-dev libglew-dev libceres-dev libcurl4-openssl-dev \
    libssl-dev libopenblas-openmp-dev \
    && mkdir -p /usr/include/opencv4 \
    && rm -rf /var/lib/apt/lists/*
# This stage depends only on the pinned source and build configuration, so app
# changes reuse the completed wheel through CI's mode=max BuildKit cache.
RUN git clone https://github.com/colmap/colmap.git /colmap \
    && git -C /colmap checkout bd1fcf654d2dd8fefa1466999c190a246f83f4b9
ARG BUILD_JOBS=2
RUN cmake -S /colmap -B /colmap/build -GNinja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/colmap \
    -DCUDA_ENABLED=ON -DCMAKE_CUDA_ARCHITECTURES=all-major \
    -DGUI_ENABLED=OFF -DONNX_ENABLED=OFF -DCGAL_ENABLED=OFF \
    -DLSD_ENABLED=OFF -DTESTS_ENABLED=OFF -DCOLMAP_HASH_MAP_BACKEND=STD \
    -DBLA_VENDOR=OpenBLAS \
    && cmake --build /colmap/build --target install --parallel "${BUILD_JOBS}"
ENV CMAKE_PREFIX_PATH=/opt/colmap LD_LIBRARY_PATH=/opt/colmap/lib:/usr/local/cuda/lib64 \
    CMAKE_BUILD_PARALLEL_LEVEL=${BUILD_JOBS}
RUN --mount=type=cache,target=/root/.cache/uv \
    cd /colmap && uv build --wheel --out-dir /tmp/wheels \
    && uvx --from auditwheel==6.8.2 auditwheel repair --plat manylinux_2_39_x86_64 \
       --exclude libcuda.so.1 --wheel-dir /wheels /tmp/wheels/*.whl

FROM registry.access.redhat.com/ubi10/ubi-minimal:10.2 AS base
COPY --from=ghcr.io/astral-sh/uv:0.13.5 /uv /usr/local/bin/uv
ENV UV_PYTHON=3.14 UV_PYTHON_INSTALL_DIR=/opt/python UV_LINK_MODE=copy
# Native Git dependencies need a compiler; PyCOLMAP also links X11 libraries.
RUN microdnf install -y --setopt=install_weak_deps=0 gcc-c++ git-core libSM libX11 libXext \
    && microdnf clean all
RUN uv python install 3.14
WORKDIR /app
COPY pyproject.toml uv.lock .python-version ./

FROM base AS cuda-12.6
RUN --mount=type=cache,target=/root/.cache/uv uv sync --locked --no-dev

FROM base AS cuda-13.0
COPY --from=pycolmap-cuda13 /wheels /opt/wheels
COPY docker/cuda13.py /opt/cuda13.py
RUN uv run --no-project python /opt/cuda13.py
RUN --mount=type=cache,target=/root/.cache/uv \
    uv lock && uv sync --locked --no-dev

FROM cuda-${CUDA_VERSION} AS runtime
ARG SHRIMPLY_SERVER_GIT_HASH=""
ENV SHRIMPLY_SERVER_GIT_HASH=${SHRIMPLY_SERVER_GIT_HASH}
COPY src ./src
COPY scripts ./scripts
CMD ["/app/.venv/bin/python", "src/main.py"]
