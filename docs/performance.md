# Cross-machine performance

The runtime keeps protocol 2 and negotiates optional capabilities. Upgrade server, worker and CLI/MCP frontend together to use the complete fast path. Existing clients retain file chunks and inline task images; old workers use the previous sync protocol. QUIC remains the default; optional TCP/TLS is described below. No mode automatically replays a submitted action.

## Directory synchronization

New peers advertise `sync_stream_v1`. They negotiate a manifest digest, send a changed manifest in bounded pages, then stream file headers and raw bytes over the job's bidirectional QUIC stream. Source confirmation sends a digest instead of a second complete manifest. Identical manifests are not retransmitted. A transfer failure resets that job's streams, not the worker connection. Legacy peers still use per-file unidirectional streams.

Peers additionally negotiate `sync_delta_pack_v1`. Files of at least 4 MiB can reuse verified 1 MiB blocks from the receiver; the sender falls back to a complete file when at least 75% of blocks changed. Signatures are capped at 8192 blocks per file and 32 MiB per job. Files up to 64 KiB are compressed into zlib batches of at most 128 files / 4 MiB uncompressed. Every installed file still receives full checksum verification and durable publication. This reduces network payload; it does not remove per-file disk durability costs. `sync.bytes` reports actual file/pack payload, while `reused_bytes` and `packs` report reuse and batching. Set raw sync argument `optimized_sync: false` for an A/B comparison. Unnegotiated peers retain full-file transfer.

Both ends cache file hashes using device/inode, size, mode, mtime and ctime including nanoseconds. Each entry expires after ten minutes. Restart starts with an empty cache. Every round still checks directory metadata; this is not an immutable filesystem snapshot. `macrun sync --strict` (or `sync_start` with `strict: true`) bypasses hash caches on both new peers. Use it for filesystems whose metadata cannot be trusted, including virtual/shared mounts that refresh stat metadata lazily. A host edit must first be visible in the source operating system; `--strict` forces content reads but cannot guarantee an external filesystem snapshot. Existing `sync::scan` callers retain strict behavior.

`sync --watch` uses native filesystem notifications with the configured interval as a coalescing delay, plus a strict ten-minute reconciliation. It falls back to polling if the watcher cannot start. Notification errors/overflow trigger a rescan; failures are retried. Events arriving during a sync remain pending. Changes in excluded directories can cause an extra metadata scan but do not transmit excluded files. Watch does not start builds.

The worker offloads hashes and filesystem mutation phases to a bounded blocking pool. Progress persists at most every 250 ms plus completion, while file data and terminal/commit records remain durable. Sync results expose `prepare_ms`, `commit_ms`, files and raw payload bytes. Server phases include source scanning. These counts do not include QUIC/JSON overhead.

## Stable workspaces

Commands, sync and file writes acquire process-wide exclusive path leases. Leases recognize existing symlink aliases and ancestor/descendant overlap. Independent roots may sync concurrently, including through different server profiles. A conflicting operation returns/fails with `busy`; it does not silently interleave mutations. Separate worker processes and unrelated local applications are outside this coordination boundary.

Successful sync returns `workspace_root` and `generation` (the committed manifest digest). For a build that must use that version:

```sh
macrun exec --cwd /Users/YOU/work/app \
  --workspace-root /Users/YOU/work/app --generation RETURNED_GENERATION \
  --wait-ms 250 'make test'
```

The command checks the generation after acquiring its lease. A new/failed sync invalidates the old generation until a successful commit. Without `workspace_root`, the command leases its cwd; specifying the project root also covers sibling source directories. The lease lasts through process completion. Shell commands must respect their declared workspace; Macrun cannot prevent an arbitrary command or external editor from writing elsewhere. Generation denotes the last synchronized input, not a sandbox or a guarantee against external writes. Use separate output/cache directories and avoid changing source during a build.

There are four concurrent shell execution slots per worker process, eight binary transfer slots and a 128-active-task admission limit per Engine. Shell capacity waiting is cancellable and counts against its timeout. These conservative limits bound contention; tune them against actual build workloads before widening them. Backends share the `desktop` resource group by default. Independent resources can opt into separate groups as described below.

Opt into a private committed-input copy with `exec --snapshot --workspace-root ROOT --generation DIGEST --cwd ROOT COMMAND` (`exec_start` accepts `snapshot: true`). Copying verifies the committed manifest, hashes and internal links under the original workspace lease. The command then leases its task-owned copy, allowing the source mirror to synchronize again. Copies are never hard links; command writes cannot modify the original through shared file inodes. The copy is retained with its task and removed by task retention. Keep build caches at explicit independent paths if you need reuse across snapshot builds. This is filesystem input isolation, not an OS sandbox: arbitrary shell commands can still address paths outside their cwd.

## Binary files and recovery

New server and worker advertise `binary_transfer_v1`. CLI upload/download automatically use raw streams with bounded buffers, checksums, backpressure, a 30-second I/O idle timeout and a one-hour overall worker/server transfer deadline. A successful transfer creates one worker task instead of one task per 512 KiB. Small `file_read/file_write` MCP calls retain their old interfaces.

Upload writes a private staging file next to the destination and publishes it by rename only after full BLAKE3 verification and fsync. Failed uploads report an ID; repeat the same file, destination and ID to resume:

```sh
macrun upload --transfer-id RETURNED_UUID ./build.zip /Users/YOU/work/build.zip
```

The receiver binds the ID to path, size and expected hash; the sender verifies the retained prefix hash before sending the remainder. Reusing an ID for different content is rejected. A checksum mismatch removes the invalid staging data; interrupted explicitly identified uploads retain `.macrun-part` and `.macrun-meta` files for retry. Worker history cleanup removes registered abandoned staging files after 24 hours, provided they are inactive and their identity journal matches. Unregistered files are never guessed or deleted. Receiver incoming UUID files older than 24 hours are cleaned at the next sync; terminal server sync jobs expire after 30 days (up to 1000 removals per hourly pass). Successful publication removes staging metadata. A retry after completed publication is safe for the file contents but is not a general exactly-once transaction.

Use `macrun download --resume REMOTE LOCAL` to retain a private partial file and source identity journal across interrupted downloads. A retry validates the remote version and the complete retained prefix before appending. The destination is replaced only after size/checksum verification and fsync. Source changes or local partial-file edits cause a failure; remove the partial/journal or choose another destination to restart. The adjacent lock file is deliberately retained so concurrent retries cannot acquire different lock inodes. Downloads without `--resume` retain their original restart behavior. The range protocol exposes `offset`, `version` and `prefix_hash`. Older endpoints fall back to sequential 512 KiB chunks; they cannot resume this new upload protocol.

## Tasks and images

`exec_start` and `mcp_call` accept `wait_ms` up to 1000: return a terminal result if ready, otherwise return the existing task identity/state. CLI exec exposes `--wait-ms`. `task_get` accepts `wait_ms` up to 25000 and `include_result: false`; the latter is useful for status-only polling. Wait/transport options are excluded from submission fingerprints, so changing a wait budget does not replay a task. Always reuse the original request UUID after uncertain delivery.

New image results are stored as task-scoped, content-addressed binary artifacts. Task metadata keeps references, MIME type, byte count and timestamps. `task.get` retains legacy inline images by default; `artifact_refs: true` returns references and `include_result: false` omits the result body. Existing desktop task detail still receives inline images on demand.

The updated MCP frontend negotiates reference responses, downloads artifacts as raw bytes and keeps a bounded 32 MiB in-memory image cache. Repeated image retrieval within that frontend avoids another worker image transfer, but MCP still delivers standard image content to the model. Cache lifetime ends with the frontend; task metadata and observation timestamps are not cached, and a cached image is not a new screenshot. Artifacts are removed with their task's retention cleanup; references from an expired task are unavailable.

The frontend handles up to 16 simultaneous JSON-RPC requests and responds by ID, so a long task wait does not block a ping or another tool. Backends remain persistent. Desktop lock waiting, backend lock waiting and backend execution share one deadline; approval has its separate 60-second budget. Timeout before dispatch is distinguished from an uncertain effect after dispatch. No batching bypasses local approvals.

## Resource groups and bounded UI sequences

Map independent backend resources in worker TOML; omitted names share `desktop`:

```toml
[resource_groups]
simulator_a = "simulator-a"
simulator_b = "simulator-b"
```

Only use different groups when the physical resources really are independent. The same group name coordinates across server profiles within one worker process. Each backend also retains its own serialization lock. Results expose resource-group and queue-time metadata.

`desktop_sequence` submits one to sixteen steps to one backend session under a shared resource lease. It requires the normal UUID, server and discovery session. A step contains `tool`, optional `arguments`, and optional `expect: {pointer, equals}` evaluated against that step's backend result. Use the final step for a fresh observation. Each child operation uses normal discovery, policy, approval, local-input yield and deduplication. Failure/condition mismatch stops later steps; timeout/cancel stops the current child and never automatically retries uncertain effects. Child IDs are deterministic, and task history records each step separately. The sequence deadline defaults to 120 seconds, capped at 600, including approval and queue waits. A sequence is not a transaction and does not roll back earlier clicks. Coordinate stale screen bounds/element IDs using the backend's own observation/session contract.

## History and transport

A durable, redacted `history-index.json` caches task summaries. Durable task writes update the shared index and invalidate its sorted in-memory view. Warm queries reuse that view; pagination copies only returned rows. Restart and five-minute reconciliation check task files; corrupt checkpoints rebuild from authoritative task records. Checkpoints occur at most every 30 seconds during reads and at shutdown. Startup recovery still inspects task records, and filtering/counting remains linear in history size. Images and command output never enter the index.

The optional TCP listener uses the same pinned certificate and token authentication, TLS with `macrun-yamux/1` ALPN, and bounded multiplexed streams (64 streams / 16 MiB receive window). Enable it explicitly:

```sh
macrun serve --listen 0.0.0.0:7443 --tcp-listen 0.0.0.0:7443 --data SERVER_STATE
macrun worker --server tcp://HOST:7443 --cert cert.der --token-file token --data WORKER_STATE
# Try QUIC for five seconds, then TCP/TLS on the same host/port:
macrun worker --server auto://HOST:7443 --cert cert.der --token-file token --data WORKER_STATE
```

Bare `HOST:PORT` continues to mean QUIC only. Prefixes also work in worker profile addresses and invitation server addresses. TCP peers require the new binary; QUIC remains the rolling-upgrade path for old peers. Fallback does not accept a different certificate or skip authentication. Status reports `quic` or `tcp_tls`; TCP does not claim a QUIC RTT sample. TCP shares head-of-line blocking and currently has no per-stream priority, so use it for reachability rather than assuming lower latency.

## Validation and design choices

```sh
make check
python3 scripts/smoke.py target/debug/macrun
python3 scripts/multi-smoke.py target/debug/macrun
./scripts/cargo-local.sh build --locked --release
python3 scripts/perf-probe.py target/release/macrun > performance.json
```

Measurements and the exact validation scope are recorded in [initial validation](benchmarks/gul-215.md) and [extended validation](benchmarks/gul-215-completion.md).

The probe uses disposable loopback server/worker instances and synthetic images. It checks one-byte large-file changes, no-op sync, transfer task counts, legacy image reads, reference-only results, concurrent MCP responses and one binary artifact fetch for repeated image requests. It does not measure real screen capture or WAN behavior. Unit tests cover checksum/resume boundaries, page limits, cached metadata changes, leases, generations, deduplication and approval expiry.

Additional reproducible probes are `scripts/wan-probe.py BINARY`, `cargo run --release --example scale-probe`, and macOS-only `scripts/capture-probe.py BINARY`. The WAN probe is a seeded user-space UDP relay, not a real carrier/VPN benchmark. The capture probe requires an unlocked interactive desktop and an already authorized CuaDriver; it captures only its own Cocoa fixture window.

Two deliberate boundaries remain: sync verifies complete directory metadata before/after transfer instead of trusting dirty notifications as a commit certificate; there is no directory-wide rollback. Hashing and transferred bytes are incremental, while metadata work remains proportional to the tree. This preserves detection of source edits that arrive before delayed/missing filesystem events. Continuous video is not introduced: discrete agents already receive fresh window-scoped observations, and CuaDriver exposes capture-only, tree-only and scaled-window modes. Its optional 30-fps H.264 recorder captures the whole main display and serves recording rather than the standard MCP image-observation contract. It can be invoked through ordinary MCP when explicitly appropriate, without adding another Macrun streaming stack. QUIC control streams retain higher priority; default congestion/window settings stay unchanged pending a deployment-specific BDP measurement.
