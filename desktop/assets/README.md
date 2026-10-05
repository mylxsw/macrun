# Macrun icon

`macrun-icon.png` is the original transparent image generated with the built-in imagegen tool on 2026-10-05. It represents a connection between a remote agent and this Mac. No terminal glyph or text is used.

Prompt: “Production-ready macOS icon; two interlocking rounded links diagonally from lower left to upper right; bold smooth white strokes on a rich blue softly luminous rounded-square tile; minimal, modern, restrained depth; transparent padding; no text, letters, terminal prompt, code chevrons, mouse pointer, Apple logo or mockup.”

The Tauri icon converter produces `desktop/src-tauri/icons/` sizes and the macOS ICNS; `desktop/src/assets/macrun-icon.png` is the 128 px UI variant. `src-tauri/src/tray.rs` draws the matching monochrome linked symbol with antialiasing and macOS template tinting. Coloured attention states and pause/offline badges remain distinct.

To regenerate platform sizes, run `npx tauri icon assets/macrun-icon.png` from `desktop/`. Only desktop platform assets are tracked.
