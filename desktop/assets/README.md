# Macrun icon

`macrun-icon.png` is the transparent image generated and revised with the built-in imagegen tool on 2026-10-05. It represents a connection between a remote agent and this Mac. No terminal glyph or text is used. The revised tile is flat, with a smaller central symbol and generous padding.

Revision prompt: “Preserve the blue rounded-square tile and white diagonal interlocking-link symbol. Reduce the central symbol to approximately 48% of the tile width, centered with generous breathing room. Strictly flat solid blue and white; no bevel, embossing, raised rim, border, gradients, reflections, highlights, texture, 3D treatment or cast shadows. Transparent outer padding, tile approximately 84% of canvas width. No text or additional symbols.”

The Tauri icon converter produces `desktop/src-tauri/icons/` sizes and the macOS ICNS; `desktop/src/assets/macrun-icon.png` is the 128 px UI variant. `src-tauri/src/tray.rs` draws the matching monochrome linked symbol with antialiasing and macOS template tinting. Coloured attention states and pause/offline badges remain distinct.

To regenerate platform sizes, run `npx tauri icon assets/macrun-icon.png` from `desktop/`. Only desktop platform assets are tracked.
