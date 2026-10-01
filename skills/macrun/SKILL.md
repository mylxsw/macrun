---
name: macrun
description: Use macrun to run commands, synchronize source files, retrieve results and screenshots, and call computer-use tools on a remote Mac or Linux worker from an agent host. Use when the user asks to work through macrun or an already configured macrun connection.
---

# macrun

macrun connects an agent host to one remote worker. Prefer the configured macrun MCP tools; use the CLI on the server host when MCP is unavailable or for upload/download and continuous sync. This skill guides use of an existing connection; it does not authorize installing services, changing other MCP entries, or replacing the user's chosen computer-use backend.

## Establish where work happens

- **Agent/server host:** source checkout, `macrun.toml`, CLI, MCP frontend and server socket.
- **Worker:** remote shell commands, command `cwd`, file tool paths, GUI apps and local MCP backends.
- Read the project's instructions and existing configuration. Discover the actual socket, source directory, remote directory and backend names; do not assume example addresses or personal deployment paths.
- Call `device_status` before remote work. `connected: true` confirms the worker link, not GUI permissions or successful compilation. If offline, report the observed connection error; inspect logs/configuration within scope instead of silently switching machines or redeploying.
- macrun 0.2 has no dedicated `build`, `test`, `run`, `shot` or `ui` commands. Compose generic capabilities for the requested task. It supports one worker per server, not device selection/routing.

Read [references/cli.md](references/cli.md) only when using the CLI, diagnosing connection setup, or transferring files without MCP.

## Choose the operation

| Need | MCP tool / approach |
| --- | --- |
| Check worker | `device_status` |
| Synchronize source once | `sync_start`, then `sync_get` |
| Start a remote shell command | `exec_start`, then `task_get` |
| Inspect/cancel a task | `task_get`, `task_cancel` |
| Read/write worker files | `file_list`, `file_read`, `file_write`, `file_move` |
| View an existing worker PNG/JPEG | `file_image` |
| Discover worker MCP backends | `mcp_servers`, then `mcp_tools` |
| Call a discovered backend tool | `mcp_call`, then `task_get` |

Use the live tool schema as the authority for arguments; agent hosts may prefix tool names.

## Synchronize before executing changed source

1. Inspect the source project's `macrun.toml`. `remote_root` names a directory on the worker, not the server. If no mapping exists, use a user-specified destination or ask for the missing destination before syncing into an existing worktree.
2. Call `sync_start` with an explicit server-side `workspace` when it differs from the MCP frontend's working directory. Save its `job_id` and poll `sync_get` until `status` is `succeeded`. Do not query a sync job with `task_get`.
3. Start the requested build/test/command using `exec_start` with the worker-side `cwd`. Choose commands from the project itself; macrun does not choose an Xcode scheme or build system.

There is no implicit synchronization before commands. Uncommitted source files are included. Deletions of previously managed files propagate; unrelated worker-generated files are retained. Use separate remote roots for distinct source projects. Default exclusions include `.git`, `.build`, `DerivedData`, `.macrun`, `target`; configure other caches and local environment files explicitly.

`sync --watch` is a separate foreground CLI process, not a server feature automatically enabled by deployment. It waits the interval after each completed sync, retries after disconnects, and does not trigger builds. For a build requiring stable source, stop the watcher and pause editing or use a fixed work copy. A failed sync can leave partial changes; wait for a successful sync before treating the mirror as current.

## Handle async tasks and uncertain delivery

- Generate a UUID `request_id` before each new `exec_start` or `mcp_call`. Record it with the arguments before sending. It equals the returned `task_id`.
- The initial `accepted`/`running` response is not completion. Poll `task_get`; for long jobs use a reasonable interval (for example 1–3 seconds initially, then longer), bounded by the task deadline and user intent. Give progress updates rather than busy polling indefinitely.
- Logs combine stdout/stderr. Use returned `output.next_offset` for incremental reads; offsets count **bytes**, not characters. Each read is bounded, so continue until the available output is consumed.
- Inspect task `status`, command `result.exit_code`, and backend errors. A successful tool transport or CLI query is not proof the underlying job succeeded.
- After a lost submission response, query the original UUID. If delivery must be retried, reuse that UUID and identical arguments. A new UUID can duplicate a click, command or other effect. Conflicting parameters with the same UUID are rejected.
- Terminal task states include `succeeded`, `failed`, `cancelled`, `timed_out`, `unknown`. On `unknown`, inspect the actual files/process/UI before deciding whether a new operation is appropriate; do not automatically replay.
- Network loss does not cancel worker tasks or reset backend sessions. Worker restart marks unfinished tasks `unknown`; backend restart, cancellation or timeout can invalidate its session. Poll after reconnect rather than claiming the operation stopped.
- `task_cancel` requests cancellation; query again to confirm state. Command process groups are terminated. Shell stdin is closed, and child processes are cleaned up when the command finishes: do not use `command &` as a persistent-service installation method.

## Use computer-use backends

1. Call `mcp_servers`, select the configured backend that fits the user's task, then `mcp_tools`. Follow pagination if returned and save the backend `session`.
2. Read the discovered tool schema. Call `mcp_call` with `server`, that `session`, exact `tool`, original `arguments`, and a new UUID `request_id`. Poll `task_get` to completion.
3. For a UI action, first obtain a current window state/screenshot and target identifiers. Use the backend's snapshot/element handles as required. The backend session is **not** a Cua window snapshot ID or Cua public session label.
4. After an action, obtain a fresh observation and verify the requested effect. “AXPress succeeded” or “input sent” alone is not an outcome. Do not blindly reuse stale elements after a new snapshot.
5. On a stale or invalid backend session, call `mcp_tools` again, then re-observe the UI. Preserve existing windows and user work; do not kill unrelated apps to recover a session.

Backend calls execute serially per backend. macrun forwards original results rather than translating every backend into Cua-specific methods. Completed image blocks are surfaced by `task_get` to the agent; inspect those images when visual verification matters.

## Files and result delivery

- `file_*` paths refer to the worker. A worker `result_path` is not a local file on the agent host; use file tools or CLI download to read it.
- `file_read` and `file_write` support up to 512 KiB per chunk. For binary data use base64; for downloads track offsets and version, and reject a mixed-version result. CLI upload/download handles chunking.
- `file_write` at offset zero truncates by default. For chunked uploads use a temporary destination and explicit offsets, then `file_move` after completion; moving replaces the destination.
- `file_image` reads an existing PNG/JPEG up to 8 MiB. It does not capture the screen. Capture through the backend first; prefer the returned MCP image over copying a base64 blob into prose.
- Large task results may require reading the saved result file instead of returning everything in one tool response. Preserve task IDs and paths for later inspection.

Report which host ran the work, the task/sync result, relevant exit codes and artifact locations, and what was actually observed. Distinguish build/test success from GUI success; do not claim screenshots, app behavior, reboot recovery or an end-to-end flow that was not checked.
