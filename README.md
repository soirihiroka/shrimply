<p align="center">
  <img src="assets/icons/dev.shrimply.Shrimply.svg" alt="Shrimply logo" width="128">
</p>

# Shrimply

> you're telling me a shrimp made this video?!

<img width="1150" height="791" alt="image" src="https://github.com/user-attachments/assets/97bbadac-d930-46ae-9df8-033bd81ab6ce" />

<img width="2206" height="1584" alt="image" src="https://github.com/user-attachments/assets/1fc005bd-9fc9-45d2-afe5-ea884bb1a99f" />

Shrimply is a simple yet powerful cross platform video editor.

Shrimply is currently pre-alpha software, which means you should expect:

- Slop
- Unable to build for whatever reason
- Undocumented footguns
- Random performance regression
- Irreversible project file breakage
- Random crashes and resource leaks
- No security (Manim etc will allow for arbitrary code execution without validation)

For more information about Shrimply's features and workflows, visit the
[documentation website](https://shrimply.pages.dev).

## Contributing

Contributions to Shrimply are welcome. There are several ways to help beyond
writing code:

- Report and investigate [issues](https://github.com/soirihiroka/shrimply/issues)
- Improve the documentation
- Translate Shrimply's interface
- Test editing workflows and project importers
- Help other users

Before submitting a contribution, read the repository's
[contribution terms](CONTRIBUTING.md).

## Developer Information

### Technology Stack

Shrimply's main application is written in Rust and uses these technologies:

- **Interface**: GTK 4 and libadwaita
- **Rendering**: Skia, wgpu, Slang, and CUDA
- **Media**: FFmpeg and PipeWire
- **Compute server**: Python

### Nix development environment

The checked-in flake provides the pinned Rust nightly, CUDA toolkit, Slang, and
native dependencies for the GTK and Qt applications on `x86_64-linux`.
Initialize the required source submodules, then enter the shell:

```sh
git submodule update --init --recursive
nix develop --accept-flake-config
make dev
```

The flake requests the [NixOS CUDA binary
cache](https://wiki.nixos.org/wiki/CUDA#Setting_up_CUDA_Binary_Cache). Multi-user
Nix installations may require adding that cache to the system Nix
configuration before the daemon will trust it.

Build the default GTK package with the pinned submodule sources:

```sh
nix build --accept-flake-config
./result/bin/shrimply
```

A compatible NVIDIA driver is still required at runtime. The package currently
embeds CUDA kernels for compute capability `sm_86`.

The development shell sets the tool and library paths expected by the existing
Makefile, so commands such as `make dev` and `make dev-qt` work unchanged. It
also sets `SLANG_LIBRARY_DIR` and `SLANG_INCLUDE_DIR` to the same prebuilt
Slang release the Cargo build scripts would otherwise download, so builds stay
offline and reproducible.

### Finding Things to Work On

Browse the [open issues](https://github.com/soirihiroka/shrimply/issues) for
reported bugs and planned work. Comment on an issue before starting a larger
change so its scope can be discussed first.

## License

Shrimply is licensed under the GNU General Public License, version 3 or later.
See [LICENSE](LICENSE).

Shrimply itself is free software, but some features depend on components that
are not free software. These include NVIDIA's [CUDA Toolkit and display
driver](https://docs.nvidia.com/cuda/eula/), [OptiX
SDK](https://developer.nvidia.com/designworks/optix/download), [Optical Flow
SDK](https://developer.nvidia.com/optical-flow-sdk), and [Video Codec
SDK](https://developer.nvidia.com/video-codec-sdk), as well as separately
licensed model weights. Those components retain their own license terms; see
the [license documentation](docs/source/licenses.rst) and
[third-party notices](THIRDPARTY.md) for details.

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=soirihiroka/shrimply&type=Date)](https://www.star-history.com/#soirihiroka/shrimply&Date)
