# Screenshot optimization

Macrun optimizes embedded CuaDriver `get_window_state` and `get_desktop_state`
images on the worker **before artifact storage and network transfer**. Defaults:

- Fit within **1920 × 1080 pixels**, preserve aspect ratio, never upscale or crop.
- `auto` uses lossless PNG encoding to preserve UI text. Existing small images
  are kept when re-encoding would increase size.
- `jpeg` opts into quality 90 for photos/video. The smaller of PNG and JPEG wins;
  transparent images remain PNG. This mode is lossy, including at quality 100.
- `original` bypasses Macrun resizing and re-encoding for one observation. It
  returns the backend's original output, still subject to CuaDriver's own limits.

For windows, Macrun first caps CuaDriver's `max_dimension` at the larger configured
edge, preserving any smaller caller cap. It then enforces both width and height.
For example, 3024 × 1964 fits into approximately 1662 × 1080. Portrait images fit
the same bounding box; dimensions are not swapped. File-output captures
(`screenshot_out_file`), `file_image`, other backends and other tools' images
(including `zoom`) are not resized by this adapter.

## Calls

The mode belongs to the Macrun request, outside backend `arguments`:

```json
{
  "server": "computer",
  "session": "SESSION_FROM_MCP_TOOLS",
  "tool": "get_window_state",
  "arguments": {"pid": 123, "window_id": 456},
  "screenshot_mode": "auto",
  "request_id": "NEW_UUID"
}
```

`mcp_call` / CLI `mcp.call` accept `auto`, `png`, `jpeg`, and `original`.
`png` forces PNG even when the source was JPEG. A `desktop_sequence` step accepts
the same `screenshot_mode` and `coordinate_task_id` fields.

An optimized result includes `macrun_screenshot`: returned/source pixel dimensions,
per-axis conversion factors, source/output bytes, actual MIME, source geometry
when available, and the screenshot task ID. `structuredContent.screenshot_width` and `screenshot_height`, plus matching JSON
text metadata, describe the returned image. `screenshot_scale` retains CuaDriver's
native backing scale (it is not necessarily the image/window size ratio). AX
element frames and window bounds keep their native coordinate spaces.
`pixels_per_point_x/y`, when window bounds are available, give the actual returned
image/window point ratios. Pixel rounding can make the two axis factors slightly different;
use the exact factors for geometry calculations rather than assuming one Retina ratio.

## Pixel actions

Use coordinates from the **latest returned screenshot**, with the same CuaDriver
session label and exact target (`pid` + `window_id`, or the primary desktop).
Macrun automatically maps `x/y`, drag endpoints and zoom crop coordinates back to
the backend's cached image frame. CuaDriver then handles Retina/window-to-screen
conversion. Do not manually apply `to_backend_x/y` before sending through Macrun.

For concurrent clients, include `coordinate_task_id` from `macrun_screenshot.task_id`
on the next pixel action. A newer observation of that target causes the old ID to
be rejected before dispatch. Reading an old task does **not** make its frame current.
Captures and coordinate conversion run under the existing backend/resource locks;
restarting the backend discards the mapping with its session.

```json
{
  "server": "computer",
  "session": "SESSION_FROM_MCP_TOOLS",
  "tool": "click",
  "arguments": {"pid": 123, "window_id": 456, "x": 830, "y": 540},
  "coordinate_task_id": "SCREENSHOT_TASK_UUID",
  "request_id": "NEW_UUID"
}
```

AX token/index actions are unchanged. `zoom` accepts coordinates from the latest
window image; its returned crop is unchanged, and `from_zoom:true` actions use
CuaDriver's existing crop conversion without a second Macrun conversion. Do not
attach a full-image `coordinate_task_id` to AX or `from_zoom` actions. A tree-only
or file-output observation invalidates Macrun's previous embedded-image mapping;
get a fresh embedded screenshot before a pixel action.

CuaDriver 0.26's `zoom` does not accept a session label; use its default session
for window observation followed by zoom. Explicitly named session frames are
kept separate and are never guessed across session labels.

## Worker configuration

No configuration is required when the backend executable is named `cua-driver`.
For a wrapper command, opt its backend name into the CuaDriver adapter explicitly:

```toml
[screenshots]
enabled = true
max_width = 1920
max_height = 1080
jpeg_quality = 90
cua_backends = ["computer"] # Only needed for wrappers; not arbitrary MCP servers.
```

Bounds must be 1–8192, JPEG quality 1–100. `enabled = false` restores passthrough
behavior for all backends. Decoding limits dimensions to 16384 per edge, 64 Mi
pixels, and 256 MiB decoder allocation. Optimization runs on the blocking pool,
and encoding/decoding failures invalidate the backend session instead of returning
an image with an untrustworthy pixel mapping.

## Verification and limits

`cargo test --test screenshot` covers image bounds, encoding, metadata, transparent
pixels, session/target isolation, coordinate conversion and artifact/frontend flow.
On an unlocked authorized Mac, `python3 scripts/screenshot-probe.py target/release/macrun`
compares modes using an owned Cocoa window and clicks its button using resized-image
coordinates. An optional second argument saves only that fixture's comparison images.

PNG preserves the resized pixels, but downscaling a whole desktop can still make
small text unreadable. Prefer window capture, `zoom`, or `original` for fine detail.
Neither compressed byte size nor pixel count directly establishes model token cost;
Linux-side history retention remains a separate concern.
