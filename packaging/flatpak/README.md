# CUDA cubins in the flatpak (M4)

`org.gnome.Sdk//50` has no `nvcc`, so `crates/render-cuda/build.rs` never
compiles CUDA kernels inside the flatpak sandbox. Instead the `.cubin` files
are built outside the sandbox and vendored as pinned sources — this is the
decided M4 plan (see `TODO.md`), not a stopgap.

## How it works

- `crates/render-cuda/build.rs` checks
  `crates/render-cuda/prebuilt/<CUDA_TARGET>/<module>.cubin` before running
  slangc/nvcc for that module. If the file exists it's copied straight to the
  expected build output path and nvcc is never invoked for that module. This
  applies both inside and outside the sandbox — it's a plain file-existence
  check, not flatpak-specific.
- Outside the sandbox, cubins are produced with the Fedora/CUDA-toolkit image
  the top-level `Dockerfile` already uses for the full app build, but via a
  leaner, separate image: `packaging/docker/cuda-artifacts.Dockerfile`. That
  image stops right after the CUDA toolkit + slang toolchain are installed —
  it never runs the full `make release qt-release` or the runtime-bundling
  steps in the top-level Dockerfile.

## Rebuild recipe

```
make flatpak-cuda-vendor
```

Don't call `cuda-artifacts-image` directly on a fresh clone — it has no
`flatpak-submodules` prerequisite of its own, and the Docker build's `COPY .
.` step will hard-fail with a "Git submodules are missing" error if
`external/slang` hasn't been initialized yet. `flatpak-cuda-vendor` inits the
submodules it needs first, then only actually runs `cuda-artifacts-image` if
`crates/render-cuda/prebuilt/$(CUDA_TARGET)/` is empty — see "Status" below.
It builds the image, runs `make cuda-artifacts` inside a container, and
copies `.slang-artifacts/cuda/sm_86/*.cubin` out to
`crates/render-cuda/prebuilt/sm_86/`. It's deliberately not part of `make
check`/`make dev`/`make cuda-artifacts` — it needs Docker, a full CUDA
toolkit download, and a full slang compile, so it's an explicit, occasional
operation for regenerating the vendored cubins (e.g. after a kernel or slang
change; delete `crates/render-cuda/prebuilt/sm_86/` first to force it). Two
host-environment fixes were needed to get real nvcc output out of this
image, both in the code/Docker layer, not just this one build:
- `packaging/docker/cuda-artifacts.Dockerfile`'s `CMD` overrides
  `CUDA_HOST_CXX=g++`: the Makefile's own default (`g++-15`) doesn't exist as
  a binary on Fedora 44 (or on this host) — only unversioned `g++` (GCC 16) is
  installed.
- `crates/render-cuda/build.rs`'s nvcc invocation now always passes
  `-allow-unsupported-compiler`, since CUDA 13.3 refuses GCC >15 without it.
  This affects every nvcc invocation, not just the flatpak/Docker path.

## Status: not committed, regenerated and cached in CI

Real cubins are produced by the recipe above and land at
`crates/render-cuda/prebuilt/sm_86/*.cubin` (~4.7MB total), gitignored
rather than committed to git or pinned as separate flatpak `sources:` —
`build.rs`'s existence check picks up whatever's on disk and treats it like
any other source file, no dedicated CUDA module or sha256 pinning needed, so
this works whether the files were vendored by hand or restored from a cache.
`.github/workflows/flatpak.yml` restores `crates/render-cuda/prebuilt` from
an `actions/cache` entry (keyed on the shader sources, `build.rs`, and the
cuda-artifacts Dockerfile) before `make dist` runs; on a cache miss,
`flatpak-cuda-vendor`'s empty-directory check triggers the Docker rebuild
same as a fresh local clone would. The tradeoff: `build.rs`'s existence
check is unconditional (in-sandbox or not), so a normal host `make
cuda-artifacts` will silently reuse whatever's vendored on disk too. If you
change a `.slang` shader, delete the corresponding
`crates/render-cuda/prebuilt/sm_86/<module>.cubin` before rebuilding, or
nvcc never runs and you get stale output.
