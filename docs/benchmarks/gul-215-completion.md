# GUL-215 extended implementation and validation

This extends the initial [stage 1–3 measurements](gul-215.md). The earlier report is historical: its full-file delta and deferred-feature descriptions do not describe this implementation. Usage and compatibility contracts are in [performance.md](../performance.md).

## Delivered changes

| Area | Implementation | Verification |
|---|---|---|
| Large-file changes | Negotiated 1 MiB block reuse, whole-file fallback, bounded signatures | Changed-base rejection, complete output hash, 64 MiB one-byte edit; three impaired-network profiles |
| Small files | Bounded zlib packs, batched signature requests, per-file durable install | Corruption/path/size limits, compressible 1000-file dataset, alternating pack/full-stream comparison |
| Download recovery | Versioned partial files, prefix verification, stable local lock, atomic publish | Interrupted transfer resumes; tampered prefix preserves original output; real CLI on QUIC/TCP/auto |
| Build input | Opt-in committed-manifest copy, no hard links, source lease released after copying | Original remains unchanged; stale/tampered/escaping input rejected; modes/directories/links preserved |
| Backend scheduling | Named process-wide resource groups, conservative shared-desktop default | Deterministic two-backend barrier and same-resource exclusion across profiles |
| UI sequences | At most 16 ordered steps, resource lease, normal child approvals, condition/error stop | Deduplication, approval/cancellation, stale session, backend crash, timeout and unknown effects |
| History | Durable redacted index, event invalidation, shared sorted snapshots | Corrupt-index rebuild, updates/deletion/redaction, 1k/10k/100k history probe |
| Cleanup | Owned upload staging and sync-incoming expiry; terminal sync-job expiry | Active leases/recent/unowned files preserved; symlinked private directories rejected |
| Restricted networks | Opt-in TLS/Yamux, forced TCP or QUIC-first fallback | Real TCP pairing, wrong certificate/token, full smoke/reconnect and UDP-unavailable fallback |

A fault-injection test caught a partial-file visibility race: Tokio may still have a filesystem write pending when the next network read fails. Both upload and resumed download now drain pending writes before returning the retained offset. Publication retains the workspace lease inside its blocking operation, so cancellation cannot release it before an already-started rename finishes.

## Local verification

- 70 core Rust tests pass, including 15 extended-feature tests and a TCP pairing/authentication test. Format and warning-free Clippy pass.
- Core LLVM coverage: **85.70% lines / 81.67% regions / 85.06% functions**. Branch coverage was not collected; region coverage is not branch coverage. Counts exclude dependencies, include core tests plus real instrumented smoke/multi-profile/performance runs, and are recorded in the JSON evidence.
- Desktop: 34 Rust tests, 28 Node tests and 112 UI tests pass; TypeScript/Vite build, desktop Clippy, managed-worker IPC smoke and the debug `.app` bundle build pass. Single-server desktop startup preserves explicit TCP/auto prefixes through address resolution.
- QUIC, TCP/TLS and automatic fallback pass commands, env/exit/cancel/timeout/dedup, sync/deletion/watch, upload/download/resume, MCP images/session/error, server reconnect and worker restart.
- Many-to-many routing/profile isolation/global-stop smoke passes. Linux amd64 server → Mac arm64 worker passes with the new server and the legacy `macrun:generic-demo` server.

CI results belong to the PR checks for the final commit. No statement here treats a queued CI run as a passing result.

## Network A/B evidence

`scripts/wan-probe.py` uses a seeded user-space UDP relay on one Mac. Each direction has independent packet loss, half the configured RTT, a serialized bandwidth queue and bounded queued packets. The same debug binary runs both modes; only `optimized_sync` changes. A 16 MiB random file changes one byte per sample. Each mode has three samples per profile. Numbers are sample medians, not production estimates or statistically established tail improvements. Control RPC p95 is calculated separately inside each sample while sync is active. CPU/filesystem activity elsewhere on the host can affect timings; no process RSS, syscall/fsync count, production VPN/MTU, 1 Gbit/s path or kernel traffic-shaper claim is made.

| RTT / bandwidth per direction / loss | Full-file median | Delta median | Median total UDP bytes, full → delta | Concurrent control p95 range, full → delta |
|---|---:|---:|---:|---:|
| 20 ms / 100 Mbit/s / 0% | 2102.50 ms | 1376.87 ms | 17,444,387 → 1,105,289 | 94.84–97.56 → 30.26–30.32 ms |
| 100 ms / 100 Mbit/s / 0.1% | 25,974.16 ms | 2913.65 ms | 17,356,747 → 1,093,854 | 124.40–151.86 → 110.76–120.37 ms |
| 200 ms / 10 Mbit/s / 1% | 192,545.44 ms | 17,509.12 ms | 17,798,570 → 1,121,606 | 352.88–392.47 → 268.42–362.95 ms |

Every sample verifies source/mirror equality. Body transfer is exactly 16 MiB versus 1 MiB, with 15 MiB reused. UDP totals include both directions, protocol headers, retransmits and concurrent control traffic; they are not application payload counts. Lossy profiles are intentionally expensive and can take several minutes to reproduce. TCP fallback is tested separately by making the selected UDP port unavailable; these impairment numbers are QUIC measurements.

## Loopback and scale

The release loopback probe verifies a 64 MiB one-byte edit transfers **1 MiB** and reuses **63 MiB** (295.01 ms in the recorded run). Mixed-tree no-op samples are 33.48–34.15 ms with zero payload. A 64 MiB upload/download creates one worker task each (329.00 / 283.35 ms). These are behavioral/single-run measurements, not p95 claims.

One thousand compressible 4 KiB files plus config use 8 packs / 19,012 payload bytes rather than approximately 4 MiB. The recorded initial install still took 8066.01 ms: per-file fsync and filesystem work remain, so reduced bytes must not be presented as proof of a local-disk speedup. A separate six-run alternating comparison measured full-stream median **2822.94 ms** versus pack median **2477.76 ms**, with changed-file bodies falling from 4,096,000 to 18,934 bytes. This also illustrates how much host/filesystem conditions affect absolute timing. Raw samples are included in the JSON evidence.

`examples/scale-probe.rs` generates disposable files and task histories. Warm history reads reuse an immutable sorted view; pagination still filters/counts records. The 100k case includes substantial local filesystem costs and ran on a host also doing builds, so use these results to locate scaling work, not predict an isolated machine's capacity.

| Records/files | Warm history view median | History page median | Warm source metadata scan | One-edit source scan |
|---|---:|---:|---:|---:|
| 1,000 | 0.0056 ms | 0.239 ms | 3.53 ms | 3.27 ms |
| 10,000 | 0.0082 ms | 2.502 ms | 42.71 ms | 41.54 ms |
| 100,000 | 0.0245 ms | 49.427 ms | 9269.94 ms | 7891.91 ms |

Cold index construction / restart reconciliation at 100k were 70.29 / 26.79 seconds in that run. Startup still reconciles task files. The index eliminates repeated filesystem scans between reconciliations; it does not make startup or filtering constant-time.

## Explicit boundaries and the remaining environment gate

The watcher triggers/coalesces work, and hash caches avoid reading unchanged content. **Directory metadata is still checked across the complete tree before and after sync.** The proposed dirty-path-only metadata scan is not implemented: relying solely on delayed/lost notifications would weaken source-change validation. Strict periodic reconciliation remains. A single-file edit therefore does not yet have constant metadata cost on a 100k-file tree. Sync still has no directory-wide rollback; snapshot execution isolates committed input instead.

Real CuaDriver 0.26.0 integration was attempted against an owned 1000×700 Cocoa fixture. The backend returned `px_capture_unavailable` and no usable image. A read-only OS check confirmed **`CGSSessionScreenIsLocked=true`**. Permissions alone do not make capture available while locked. Real screenshot latency/size, AX-versus-capture-only comparisons and GUI interaction acceptance are therefore **blocked, not passed**. The reproducible probe preflights the lock and exits with status 2; it must be rerun on an unlocked interactive desktop. It never records unrelated windows or changes the user's daemon policy. The fixture compiled, but no production Xcode project's cold/warm-cache claim is made.

Continuous video is a deliberate non-addition, not an implemented/tested feature: CuaDriver already has capture-only, tree-only and resized-window observations, while its optional 30-fps H.264 recorder captures the whole main display. Recording is distinct from standard MCP image observation. No new always-on recorder, codec or WebRTC stack is introduced. The existing generic MCP path can invoke backend recording tools when explicitly needed.

## Reproduce

```sh
make check
MACRUN_TEST_TRANSPORT=quic python3 scripts/smoke.py target/debug/macrun
MACRUN_TEST_TRANSPORT=tcp python3 scripts/smoke.py target/debug/macrun
MACRUN_TEST_TRANSPORT=auto python3 scripts/smoke.py target/debug/macrun
python3 scripts/multi-smoke.py target/debug/macrun
python3 scripts/desktop-smoke.py target/debug/macrun
./scripts/cargo-local.sh build --locked --release
python3 scripts/perf-probe.py target/release/macrun
python3 scripts/pack-probe.py target/release/macrun
python3 scripts/wan-probe.py target/debug/macrun
./scripts/cargo-local.sh run --locked --release --example scale-probe
# Requires an unlocked macOS desktop and an already authorized CuaDriver:
python3 scripts/capture-probe.py target/release/macrun
```

Raw measurements and coverage summaries: [gul-215-completion.json](gul-215-completion.json). The original stage 1–3 baseline pairs are kept separately for traceability.
