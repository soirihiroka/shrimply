<p align="center">
  <img src="assets/icons/dev.shrimply.Shrimply.svg" alt="Shrimply logo" width="128">
</p>

# Shrimply

> you're telling me a shrimp made this video?!

<img width="1150" height="791" alt="image" src="https://github.com/user-attachments/assets/97bbadac-d930-46ae-9df8-033bd81ab6ce" />

Shrimply is a free and open-source video editor for creating videos from start
to finish, whether you are making a quick edit or something fancy.

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

### Building a Flatpak

```
make dist
```

This installs flatpak, flatpak-builder, the GNOME runtime/SDK, and required
SDK extensions; initializes the submodules the build needs; regenerates the
vendored CUDA cubins if missing (needs Docker); and builds
`packaging/flatpak/dev.shrimply.Shrimply.yaml` from scratch into a
single-file `dev.shrimply.Shrimply.flatpak` bundle. It's a long build
(FFmpeg, OpenCV, and the whole Rust workspace all compile from source) — see
`packaging/flatpak/README.md` for details, caching notes, and the manual
step-by-step equivalent.

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
