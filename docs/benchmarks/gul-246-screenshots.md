# GUL-246 screenshot validation

Validated on macOS with CuaDriver 0.26.0 using
`python3 scripts/screenshot-probe.py target/release/macrun` and the repository's
owned Cocoa fixture. No other application window or whole desktop was captured.

| Mode | Returned dimensions | Encoded bytes |
| --- | --- | ---: |
| Original backend PNG | 1568 × 1148 | 1,722,470 |
| Default optimized PNG | 1475 × 1080 | 1,207,126 |
| JPEG requested, transparency-preserving PNG fallback | 1475 × 1080 | 1,207,126 |

The default saved **29.9%** in this fixture. The window has transparent rounded
corners, so the JPEG option correctly retained PNG. The synthetic opaque-image
test separately verifies JPEG selection and decoding. This is a single-window
functional measurement, not a general compression ratio or latency benchmark.
The optimized fixture's small monospaced text was visually inspected and remained
readable; this does not guarantee readability for all fonts or larger desktops.

The probe computes the test button's center in the returned image, submits a
pixel click with `coordinate_task_id`, then obtains a fresh AX tree and verifies
the label changed to `Macrun fixture action confirmed`. This tests the actual
pixel path rather than relying on an AX token click or dispatch acknowledgement.

One backend detail matters: in this run CuaDriver reported `screenshot_scale: 2`
even though its returned image was 1568 pixels wide for a 1000-point window.
Macrun therefore preserves that native backing-scale field and provides separate
`pixels_per_point_x/y` plus exact `to_backend_x/y` factors. Multiplying by the
Retina scale alone is incorrect once the backend has already reduced the image.

Automated coverage includes landscape/portrait/extreme aspect ratios, no upscaling,
original bytes, PNG/JPEG selection, transparency, corrupt and ambiguous images,
metadata consistency, backend/session/target isolation, stale-task rejection,
click/drag/zoom conversion, sequence overrides, artifact storage and independent
MCP image output with no Base64 in text summaries. Unknown image tools pass through.

See [configuration and usage](../screenshots.md) for opt-out and original/zoom access.

Local checks: 100 Rust tests passed across all targets; `cargo clippy --all-targets
-- -D warnings`, `cargo fmt --check`, release build and `scripts/smoke.py` passed.
`cargo llvm-cov --all-targets` measured the new `src/screenshot.rs` at 98.0% line
coverage and 89.3% function coverage. Repository-wide line coverage was 66.8%; the
98.0% figure is specifically the new screenshot module, not a repository-wide claim.
