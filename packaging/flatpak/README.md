# CUDA cubins in the flatpak

`org.gnome.Sdk//50` has no `nvcc`, so the CUDA kernels can't be compiled inside
the flatpak sandbox. Instead the `.cubin` files are built outside it and
vendored on disk.

## How it works

- `crates/render-cuda/build.rs` checks
  `crates/render-cuda/prebuilt/<CUDA_TARGET>/<module>.cubin` before running
  slangc/nvcc. If it exists, it's copied to the build output and nvcc is
  skipped. Plain file-existence check, not flatpak-specific — it applies to
  every build.
- The cubins are produced by `packaging/docker/cuda-artifacts.Dockerfile`, a
  lean image with just the CUDA + slang toolchain (none of the top-level
  Dockerfile's app build / runtime bundling).

## Rebuild recipe

```
make flatpak-cuda-vendor
```

Inits the submodules it needs, then runs the Docker build only if
`crates/render-cuda/prebuilt/$(CUDA_TARGET)/` is empty. Don't call
`make cuda-artifacts-image` directly on a fresh clone — it has no submodule
prerequisite and the Docker `COPY . .` fails if `external/slang` isn't
initialized.

Needs Docker, a CUDA toolkit download and a full slang compile, so it's not
part of `make check` / `make dev`.

## Caveats

- The cubins are **gitignored**, not committed. `.github/workflows/flatpak.yml`
  caches `crates/render-cuda/prebuilt` (keyed on the shader sources, `build.rs`
  and the Dockerfile); on a cache miss the Docker rebuild runs as on a fresh
  clone.
- `build.rs`'s existence check is unconditional, so a plain host `make
  cuda-artifacts` also reuses whatever's vendored. **After editing a `.slang`
  shader, delete the corresponding `prebuilt/sm_86/<module>.cubin`** or nvcc
  never re-runs and you get stale output.
- `build.rs` always passes `nvcc -allow-unsupported-compiler` (CUDA 13.3
  refuses GCC >15 otherwise); the Dockerfile's `CMD` sets `CUDA_HOST_CXX=g++`
  because the Makefile default `g++-15` isn't installed on Fedora 44.
