# macOS app icon

Edit `Shrimply.icon` in Apple's Icon Composer. Its foreground layers are SVG;
the native fill, background, shadow, and appearance settings live in `icon.json`.

`make appkit-icon` compiles it with Xcode's `actool` into
`target/macos-icon/Assets.car` and the compatibility `Shrimply.icns`.
The asset catalog carries the Default, Dark, and Mono icon stacks; macOS uses
Mono for Clear and Tinted, and handles Light/Dark/Auto selection.

`make dev-mac` runs `target/debug/Shrimply.app` with these resources. Release
packaging uses the same compiled assets. Do not set `applicationIconImage` at
runtime: that would replace the system-rendered bundle icon with a static image.

See Apple's [Icon Composer guide](https://developer.apple.com/documentation/xcode/creating-your-app-icon-using-icon-composer).

## FFmpeg

AppKit build, check and release targets build FFmpeg under `target/macos-ffmpeg`.
They use the installed Homebrew FFmpeg's codec and filter choices and enable
Vorbis with Homebrew `libvorbis`. The local
patch requires a hardware VideoToolbox decoder when creating each decompression
session, including HEVC and ProRes. An unsupported stream fails to open.
