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
# Ubuntu 24.04's CMake predates CUDA 13 and all-major still includes removed GPUs.
RUN cmake -S /colmap -B /colmap/build -GNinja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/colmap \
    -DCUDA_ENABLED=ON -DCMAKE_CUDA_ARCHITECTURES="75-real;80-real;90-real;100-real;110-real;120" \
    -DGUI_ENABLED=OFF -DONNX_ENABLED=OFF -DCGAL_ENABLED=OFF \
    -DLSD_ENABLED=OFF -DTESTS_ENABLED=OFF -DCOLMAP_HASH_MAP_BACKEND=STD \
    -DBLA_VENDOR=OpenBLAS \
    && cmake --build /colmap/build --target install --parallel "${BUILD_JOBS}"
ENV CMAKE_PREFIX_PATH=/opt/colmap LD_LIBRARY_PATH=/opt/colmap/lib:/usr/local/cuda/lib64 \
    CMAKE_BUILD_PARALLEL_LEVEL=${BUILD_JOBS}
RUN --mount=type=cache,target=/root/.cache/uv \
    cd /colmap && uv build --wheel --out-dir /tmp/wheels
# Ubuntu's patchelf corrupts OpenBLAS segment mappings during wheel repair.
ADD --checksum=sha256:2abdc34fbe949c995a1baa4ec310896a085588a45a99cd0d726851bdfac2a4bb \
    https://github.com/NixOS/patchelf/releases/download/0.19.2/patchelf-0.19.2-x86_64.tar.gz /tmp/patchelf.tar.gz
RUN tar -xzf /tmp/patchelf.tar.gz -C /usr/local ./bin/patchelf
RUN --mount=type=cache,target=/root/.cache/uv \
    uv tool run --from auditwheel==6.8.2 auditwheel repair --plat manylinux_2_39_x86_64 \
       --exclude libcuda.so.1 --exclude libGLX.so.0 --exclude libOpenGL.so.0 \
       --exclude libGLdispatch.so.0 --wheel-dir /wheels /tmp/wheels/*.whl

# PyAV's public wheels contain a private FFmpeg. Build the server's wheel from
# the same checkout as the editor and bundle its runtime libraries for UBI.
FROM ubuntu:24.04 AS pyav
COPY --from=ghcr.io/astral-sh/uv:0.13.5 /uv /usr/local/bin/uv
ENV DEBIAN_FRONTEND=noninteractive UV_PYTHON=3.14
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential ca-certificates git nasm ninja-build meson pkg-config patchelf \
    libasound2-dev libdav1d-dev libffmpeg-nvenc-dev libgnutls28-dev libmp3lame-dev libopus-dev \
    libsvtav1enc-dev libvpl-dev libvpx-dev libwebp-dev libx264-dev libx265-dev \
    libxcb1-dev libxcb-shape0-dev libxcb-shm0-dev libxcb-xfixes0-dev zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*
# Keep the AMF encoder headers and VMAF filter available in the original PyAV build.
RUN git init /tmp/amf && git -C /tmp/amf sparse-checkout set amf/public/include \
    && git -C /tmp/amf fetch --filter=blob:none --depth 1 https://github.com/GPUOpen-LibrariesAndSDKs/AMF.git eae4a4b7efc35f8b0a3977a0984c0d642efc4e63 \
    && git -C /tmp/amf checkout FETCH_HEAD && cp -a /tmp/amf/amf/public/include /usr/local/include/AMF \
    && git init /tmp/vmaf && git -C /tmp/vmaf sparse-checkout set libvmaf \
    && git -C /tmp/vmaf fetch --filter=blob:none --depth 1 https://github.com/Netflix/vmaf.git 17a67b238ce0539bdeafdc95961abac64fa16ea8 \
    && git -C /tmp/vmaf checkout FETCH_HEAD \
    && meson setup /tmp/vmaf-build /tmp/vmaf/libvmaf --libdir=lib -Denable_tests=false -Denable_docs=false \
    && meson install -C /tmp/vmaf-build && ldconfig && rm -rf /tmp/amf /tmp/vmaf /tmp/vmaf-build
COPY --from=project /external/ffmpeg/ /src/ffmpeg/
WORKDIR /src/ffmpeg
ARG BUILD_JOBS=2
RUN cp RELEASE FF_VERSION && ./configure --prefix=/opt/ffmpeg --enable-shared --disable-static \
    --disable-doc --disable-debug --disable-programs --enable-gpl --enable-version3 --enable-gnutls \
    --enable-libdav1d --enable-libmp3lame --enable-libopus --enable-libsvtav1 --enable-libvmaf \
    --enable-libvpl --enable-libvpx --enable-libwebp --enable-libx264 --enable-libx265 \
    --enable-amf --enable-nvenc --enable-nvdec && make -j"${BUILD_JOBS}" && make install
WORKDIR /wheel
COPY uv.lock ./
ENV LD_LIBRARY_PATH=/opt/ffmpeg/lib:/usr/local/lib
RUN --mount=type=cache,target=/root/.cache/uv \
    export PKG_CONFIG_PATH=/opt/ffmpeg/lib/pkgconfig && \
    source_url="$(uv run --no-project --python 3.14 python -c 'import tomllib; p=next(p for p in tomllib.load(open("uv.lock", "rb"))["package"] if p["name"] == "av"); print(p["source"]["url"] + "#" + p["sdist"]["hash"].replace(":", "=", 1))')" && \
    uv tool run --python 3.14 --from pip pip wheel --no-deps --wheel-dir /tmp/pyav "$source_url" && \
    uv tool run --from auditwheel==6.8.2 auditwheel repair --plat manylinux_2_39_x86_64 \
       --wheel-dir /wheels /tmp/pyav/*.whl

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
RUN --mount=type=cache,target=/root/.cache/uv uv sync --locked --no-dev --no-install-package av

FROM base AS cuda-13.0
RUN microdnf install -y --setopt=install_weak_deps=0 libglvnd-glx libglvnd-opengl libXrender \
    && microdnf clean all
COPY --from=pycolmap-cuda13 /wheels /opt/wheels
COPY docker/cuda13.py /opt/cuda13.py
RUN uv run --no-project python /opt/cuda13.py
RUN --mount=type=cache,target=/root/.cache/uv \
    uv lock && uv sync --locked --no-dev --no-install-package av

FROM cuda-${CUDA_VERSION} AS runtime
RUN --mount=from=pyav,source=/wheels,target=/pyav \
    uv pip install --python /app/.venv/bin/python --no-deps /pyav/*.whl
ARG SHRIMPLY_SERVER_GIT_HASH=""
ENV SHRIMPLY_SERVER_GIT_HASH=${SHRIMPLY_SERVER_GIT_HASH}
COPY src ./src
COPY scripts ./scripts
CMD ["/app/.venv/bin/python", "src/main.py"]
