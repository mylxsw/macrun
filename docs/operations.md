# Running and maintaining Macrun

[English overview](../README.md) · [中文概览](../README.zh-CN.md) · [Agent installation](agent-install.md)

Examples use a server at `$HOME/.local/share/macrun-server`, a worker at `$HOME/.local/share/macrun-worker`, and binaries in `$HOME/.local/bin`. Each home belongs to the user **on that machine**. Adapt paths to your deployment. Never recreate identity files as a routine restart step.

## Operational logs

Both server and worker write one JSON line to **stderr** per handled operation, with a UTC timestamp (`time`), component, operation, request ID, outcome and elapsed milliseconds. This is enabled by default; stdout remains reserved for CLI/MCP responses. Heartbeats and individual transfer chunks are not logged.

```json
{"time":"2026-10-03T08:30:00.125Z","component":"worker","event":"operation","operation":"exec.start","request_id":"…","task_id":"…","status":"succeeded","task_status":"accepted","duration_ms":2}
{"time":"2026-10-03T08:30:02.540Z","component":"worker","event":"task_finished","operation":"exec.start","task_id":"…","status":"succeeded","duration_ms":2417,"exit_code":0}
```

`status=succeeded` on an operation means the API request succeeded. It does **not** mean an asynchronous task finished: inspect `task_status`, or find `task_finished` by `task_id`. Every explicit poll is an operation and therefore produces a line; polling less often reduces log volume. Forwarded requests share `request_id` across server and worker. Sync uses `job_id` on the server and that ID as the worker's `request_id`.

Connection, reconnection, persistence failures and interrupted-task recovery are also logged. Command text, environment values, file contents, screenshots and MCP argument/result payloads are omitted. MCP server/tool names are included. Use `task.get` for full task output and error details. A malformed sync job ID returns `invalid_argument` rather than closing the request with a UUID parser error.

Use `journalctl -u macrun-server -f` on Linux, and `tail -f` on the worker plist's `StandardErrorPath` on macOS (examples below). These logs require rebuilding and restarting **both** binaries after upgrading; editing source alone does not update running services. Configure retention through journald or your chosen file log rotation tool.

## Linux server: systemd

Use [deploy/macrun-server.service](../deploy/macrun-server.service) as a template. Before installing, replace `REPLACE_USER` and all paths. Its original binary path is `/usr/local/bin/macrun` and socket is `/tmp/macrun.sock`; change them if using the README's user-local installation and data-directory socket. Choose the intended listen interface. Make sure the service user owns the data directory.

From your rendered unit's directory, on Linux:

```bash
sudo systemd-analyze verify ./macrun-server.service
sudo install -m 644 ./macrun-server.service /etc/systemd/system/macrun-server.service
sudo systemctl daemon-reload
sudo systemctl enable --now macrun-server
systemctl status macrun-server --no-pager
```

If replacing an already running unit, explicitly `sudo systemctl restart macrun-server` after daemon-reload. Stop any foreground server using the same port/socket first.

Choose the needed operation, not the entire block:

```bash
sudo systemctl restart macrun-server
sudo systemctl stop macrun-server
sudo systemctl start macrun-server
systemctl is-enabled macrun-server
sudo journalctl -u macrun-server -n 100 --no-pager
sudo journalctl -u macrun-server -f
```

A private-network bind may require that network service to be ready; include the appropriate dependency in your own unit. Do not infer that opening a TCP port enables QUIC: the runtime uses UDP.

## macOS worker: LaunchAgent

Use [deploy/dev.macrun.worker.plist](../deploy/dev.macrun.worker.plist) as a template. Render every `REPLACE_USER`/`REPLACE_SERVER_IP`, use absolute executable/data/config paths, and ensure log directories exist. The file's `--config` assumes a valid backend configuration: remove both that argument and its following path if no backend is required.

As the logged-in GUI user, from your rendered plist directory:

```bash
mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.local/share/macrun-worker"
plutil -lint ./dev.macrun.worker.plist
# Preserve an existing customized plist before replacing it.
cp ./dev.macrun.worker.plist "$HOME/Library/LaunchAgents/dev.macrun.worker.plist"
launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/dev.macrun.worker.plist"
launchctl print "gui/$(id -u)/dev.macrun.worker"
```

If the service is already loaded, unload it before loading a changed plist. Stop a foreground test worker first. The template's `RunAtLoad` and `KeepAlive` start at graphical login and restart unexpected exits; this is not a pre-login system daemon.

```bash
# Restart a loaded worker (running tasks may become unknown):
launchctl kickstart -k "gui/$(id -u)/dev.macrun.worker"
# Stop/unload for this login:
launchctl bootout "gui/$(id -u)/dev.macrun.worker"
# Start again after unloading:
launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/dev.macrun.worker.plist"
# Inspect the configured stderr log:
tail -n 100 "$HOME/.local/share/macrun-worker/stderr.log"
```

`bootout` does not remove the plist; it can load again at the next login. Do not use ordinary process termination as a stop mechanism with KeepAlive enabled. When the plist changes, bootout/bootstrap reloads it; kickstart alone does not reread a modified plist.

If automation receives `Bootstrap failed: 5`, inspect the plist, paths, existing job and GUI domain. In a restricted automation environment, the same validated command may need to be run by the user in normal Terminal. Do not bypass execution restrictions or claim success from copying a plist.

## Computer-use backend

Backend lifecycle and permissions belong to the selected tool. For an installed CuaDriver app, inspect its current help and status. These commands were used with CuaDriver 0.26.0:

```bash
/Applications/CuaDriver.app/Contents/MacOS/cua-driver status
/Applications/CuaDriver.app/Contents/MacOS/cua-driver permissions status --json
open -g -a CuaDriver --args serve
```

If permissions are missing, the app's `permissions grant` command can guide the user through macOS authorization. Verify live capture afterward. Stop/restarting a shared daemon can affect other clients; do that only when needed for the requested work. After a backend restart, rediscover tools/session and obtain fresh UI state.

## Checks after installation or restart

On the agent host, use the actual socket:

```bash
export MACRUN_SOCKET="$HOME/.local/share/macrun-server/control.sock"
"$HOME/.local/bin/macrun" status
"$HOME/.local/bin/macrun" exec --cwd /tmp 'hostname; uname -m; echo macrun-ok'
"$HOME/.local/bin/macrun" task TASK_UUID
```

Check `connected`, terminal task state, worker identity, and exit code. For a backend, separately verify discovery and real image return. For sync, use a known scratch file and observe it remotely. A successful shell command does not establish GUI permissions.

To test autostart, schedule an authorized login/reboot test and repeat these checks afterward. Until then describe startup as configured, not reboot-tested.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| Missing Unix socket | Server process and exact `--socket`/`MACRUN_SOCKET`; default `/tmp/macrun.sock` may differ from deployment |
| Worker offline | Worker log, target IP/interface, UDP path, private-network state, matching cert/token and protocol |
| Wrong architecture / loader error | `uname -m`, selected distribution and Linux runtime libraries; build natively when needed |
| MCP entry exists but remote work fails | Client config only starts the frontend; inspect `device_status` and actual task state |
| `backend exited; session lost` | Worker stderr, backend executable/args/env/cwd, daemon, app permissions; rediscover after recovery |
| Screenshot fails despite permission flags | Perform real capture under the backend's app identity and active graphical session |
| Click acknowledged but no effect | Fresh screenshot/tree and grounded target; do not equate event delivery with UI outcome |
| `unknown` task | Inspect actual effects; do not automatically repeat side effects with a new UUID |
| Source not updating | Watch process lifecycle, workspace/config/exclusions, last successful sync and source changes during scanning |
| Old behavior after build | Distribution file may differ from installed executable; check service arguments and restart matching binaries |

Network reconnect retains worker tasks and backend sessions, but worker restart does not promise process continuation. Completed results remain in the worker data directory. Logs and task results have no automatic retention policy.

## Updates and rollback

1. Record the currently deployed commit, binary/config locations and important running tasks.
2. Build matching server and worker binaries from the intended revision; run relevant checks.
3. Retain old binaries and configuration separately. Preserve identity and task state; do not commit credentials/logs to Git.
4. Install replacements and restart only the affected services. Verify both transport and the requested business operation independently.
5. If rollback is required, restore a compatible pair of binaries/configurations and re-verify. There is no automatic rollback or guarantee that old versions can read future state formats.

0.1 → 0.2 changes the wire protocol and removes build/test/run/shot/ui-specific commands. Update both ends, migrate project config to sync-only fields, use generic worker backend `--config`, and use fresh state directories for that migration. Existing app processes are not automatically adopted or stopped.

## Continuous sync is a separate process

```bash
macrun --workspace /absolute/source/project sync --watch --interval-ms 1000
```

Keep it in a terminal, stop with Ctrl-C, or configure its own service if requested. A running server does not start watchers. Avoid duplicate watchers for one mirror. Wait for complete sync before executing against changed source; use a fixed copy if the command requires immutable inputs.
