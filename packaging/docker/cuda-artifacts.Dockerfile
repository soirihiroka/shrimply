# Leaner image for producing prebuilt CUDA cubins outside the flatpak sandbox
# (M4, see TODO.md). Mirrors the toolchain setup in the top-level Dockerfile
# (rustup, CUDA toolkit from NVIDIA's repo) but stops there: it does NOT run
# `make release qt-release` or any of the runtime-bundling steps below it in
# the top-level Dockerfile — this image only needs to run `make
# cuda-artifacts`, which builds `shrimply-render-cuda` (slangc + nvcc) and
# nothing else.
#
# Built and run via `make cuda-artifacts-image` (see Makefile). Kept as a
# separate file rather than folded into the top-level Dockerfile as a build
# stage: the two images serve different, occasional purposes and touching the
# working top-level Dockerfile to add staging risks breaking it for no
# benefit here.
FROM fedora:44
WORKDIR /src

RUN dnf install -y curl git make patchelf rustup && dnf clean all

RUN rustup-init -y --default-toolchain none --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

# deps-fedora expects the RPMFusion repos to exist even though cuda-artifacts
# itself never touches ffmpeg/rubberband/opencv etc.
RUN dnf install -y \
        "https://mirrors.rpmfusion.org/free/fedora/rpmfusion-free-release-$(rpm -E %fedora).noarch.rpm" \
        "https://mirrors.rpmfusion.org/nonfree/fedora/rpmfusion-nonfree-release-$(rpm -E %fedora).noarch.rpm" \
    && dnf clean all

COPY Makefile ./
RUN make deps-fedora DNF="dnf -y"

# CUDA toolkit from NVIDIA's own repo (Fedora's repos don't ship it)
RUN . /etc/os-release && \
    curl -fsSL -o /etc/yum.repos.d/cuda-fedora.repo \
        "https://developer.download.nvidia.com/compute/cuda/repos/fedora${VERSION_ID}/x86_64/cuda-fedora${VERSION_ID}.repo" && \
    dnf install -y cuda-toolkit && \
    dnf clean all

COPY . .

RUN rustup show

# Git submodules (slang) might be missing from the build context.
RUN test -e external/slang/CMakeLists.txt || { \
        echo "Git submodules are missing from the build context." >&2; \
        echo "Run 'git submodule update --init --recursive' before 'docker build'." >&2; \
        exit 1; \
    }

ENV CUDA_HOME=/usr/local/cuda
ENV CUDA_TOOLKIT_PATH=/usr/local/cuda

# No NVIDIA driver in the build container; stub libcuda.so.1 so nvcc's link
# step resolves (same trick as the top-level Dockerfile).
RUN ln -sf libcuda.so /usr/local/cuda/lib64/stubs/libcuda.so.1
ENV LD_LIBRARY_PATH=/usr/local/cuda/lib64/stubs:${LD_LIBRARY_PATH}

# `make cuda-artifacts` builds slang (slangc/slang-glslang) itself via the
# slang-compiler Makefile target, then compiles the kernels with nvcc.
#
# CUDA_HOST_CXX override: the Makefile's default (g++-15) assumes a
# version-suffixed binary that doesn't exist on this Fedora 44 image (or on
# this host either) — only plain `g++` (GCC 16) is installed. Overridden here
# rather than in the Makefile default since that default is a separate,
# pre-existing concern unrelated to this packaging work.
CMD ["make", "cuda-artifacts", "CUDA_HOST_CXX=g++"]
