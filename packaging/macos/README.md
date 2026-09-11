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
