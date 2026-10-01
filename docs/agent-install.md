# Install and configure Macrun — guide for coding agents

This is the installation entrypoint for Claude Code, Codex, and other agents. For day-to-day remote work after installation, load [the Macrun usage Skill](../skills/macrun/SKILL.md). Human overview: [English](../README.md) / [中文](../README.zh-CN.md).

Read this guide before changing either machine. It defines the inputs, execution locations, and evidence needed to finish installation. A successful package copy is not a working deployment.

## 1. Establish the deployment inputs

Inspect available configuration first. Ask only for values you cannot determine from the user's request or environment. Record the following without recording secret contents:

| Input | What to establish |
| --- | --- |
| Agent/server host | SSH alias or local access; OS, CPU architecture, service user |
| Worker | Local/remote access; OS, CPU architecture, GUI login user if applicable |
| Source revision | Same Macrun Git commit for both executables and the Skill |
| Connection | Server numeric IP reachable from the worker, UDP port, intended listen interface |
| Filesystem layout | Absolute binary, server data, worker data, socket and log paths |
| Agent clients | Claude Code, Codex, or another installed MCP host; existing `macrun` entries |
| Workspaces | Optional source directory on server and separate destination on worker |
| GUI backend | Optional installed stdio command, arguments, permissions and service lifecycle |
| Persistence | Foreground demo or persistent systemd/LaunchAgent installation |

Do not infer CPU architecture from “Linux.” `aarch64` maps to `linux/arm64`; `x86_64` maps to `linux/amd64`. A Mac ARM64 build does not run on Linux ARM64. Do not substitute the local machine when remote access fails.

Macrun currently accepts a numeric socket address for `--server`, not an SSH alias or hostname. Use `IP:port`, or `[IPv6]:port`. UDP must reach the server; an SSH tunnel alone does not carry QUIC. Reuse an existing reachable private network when appropriate; changing cloud firewall/network configuration is a separate concrete action, not a hidden installation step.

Inspect existing services and state before modification. Follow the user's infrastructure repository/deployment conventions if present. This guide does not grant permission to overwrite unrelated agent settings, terminate other apps, or rotate an existing identity.

## 2. Inspect and build

On each machine, inspect `uname -s`, `uname -m`, available `git`, `make`, C compiler and `macrun --version`. Inspect installed agent CLIs using their local `mcp --help`; do not assume both clients are installed.

Obtain https://github.com/mylxsw/macrun and choose one revision. With an existing checkout, inspect dirty state and preserve local changes instead of resetting it. Build that same revision for both machines.

Native build, from the repository root on each target:

```bash
make deps
make build
make install
"$HOME/.local/bin/macrun" --version
```

`make deps` requires a native C compiler. macOS uses Xcode Command Line Tools; Debian/Ubuntu uses `build-essential`, plus Git/curl/CA certificates. It bootstraps Rust only if absent and does not install desktop automation tools. Read `scripts/deps.sh` when custom toolchain locations or versions matter.

Alternative: on a Mac with Docker, build the Mac binary natively and export the Linux binary:

```bash
make build-client
make build-server PLATFORM=linux/arm64   # Or linux/amd64 after inspecting server
```

Native outputs are in `dist/<platform>/macrun`. Copy the correct file to the intended host and use `install -m 755` at its selected destination. The Docker Linux build targets Debian bookworm/glibc; verify execution on the actual destination, not just the file's existence. `make install` only installs on its own host; it does not configure services or MCP.

**Checkpoint:** each installed executable runs `--version` on its target. Keep the source commit and output paths in the completion record.

## 3. Initialize the server identity once

Use user-owned absolute paths. A conventional layout is:

```text
Server binary:  $HOME/.local/bin/macrun
Server data:    $HOME/.local/share/macrun-server
Server socket:  $HOME/.local/share/macrun-server/control.sock
Worker binary:  $HOME/.local/bin/macrun
Worker data:    $HOME/.local/share/macrun-worker
```

Each `$HOME` refers to the user on that machine. Expand variables when writing systemd/plist/MCP arguments; these files do not execute shell substitution automatically.

For a **new** server installation:

```bash
"$HOME/.local/bin/macrun" init --data "$HOME/.local/share/macrun-server"
```

If identity files already exist, reuse them. If only part of the identity exists, diagnose the incomplete state; do not delete it and initialize again blindly. Keep `key.der` server-side. Copy only `cert.der` and `token` to the worker over the available trusted file-transfer channel; restrict the copied token's file permissions. Never print or put credential contents in a completion report or source repository.

## 4. Prove foreground connectivity

On the server, run in a dedicated terminal/process:

```bash
"$HOME/.local/bin/macrun" \
  --socket "$HOME/.local/share/macrun-server/control.sock" \
  serve --listen 0.0.0.0:7443 --data "$HOME/.local/share/macrun-server"
```

Use the selected private listen IP instead when that is the deployment plan. In another terminal on the worker, replace `192.0.2.10` with the actual server IP:

```bash
"$HOME/.local/bin/macrun" worker --server 192.0.2.10:7443 \
  --cert "$HOME/.local/share/macrun-worker/cert.der" \
  --token-file "$HOME/.local/share/macrun-worker/token" \
  --data "$HOME/.local/share/macrun-worker"
```

On the server, in a separate shell:

```bash
export PATH="$HOME/.local/bin:$PATH"
export MACRUN_SOCKET="$HOME/.local/share/macrun-server/control.sock"
macrun status
macrun exec --cwd /tmp 'hostname; uname -m; echo macrun-install-ok'
macrun task TASK_UUID
```

Poll the returned task ID until terminal state. Require `connected: true`, the expected worker hostname/architecture, `status: succeeded`, `result.exit_code: 0`, and the marker in output. A CLI process exit code alone is insufficient.

**If blocked:** inspect server/worker stderr, IP/interface selection, UDP reachability, certificate/token pairing and protocol compatibility. Stop repetitive retries when the same concrete failure persists; report the failure and required external action. Do not redeploy unrelated network software or repeat tasks with new IDs just because a reply was lost.

## 5. Configure persistent services if requested

Read [operations.md](operations.md), [the systemd template](../deploy/macrun-server.service) and [the LaunchAgent template](../deploy/dev.macrun.worker.plist). Render them with the selected **absolute** paths and user names; they are examples, not ready-to-install files.

Stop your foreground test processes before starting managed replacements. Do not run two workers against the same server/state directory. For the Mac, use the graphical user's LaunchAgent rather than a root daemon if desktop interaction is needed. Ensure output log directories exist before bootstrap. Omit the two `--config`/path array entries if no backend is configured.

Verify the loaded service's actual arguments, running state and `macrun status` again. A plist written to disk does not prove it loaded. If an automation sandbox rejects `launchctl bootstrap`, finish all independent steps and give the user the precise normal-Terminal command. Do not call the deployment persistent until the installed service is verified.

Do not reboot/logout merely to test autostart unless authorized. Report “configured for login/startup; reboot recovery not tested” when that is the evidence.

## 6. Add optional computer use

Commands, files and synchronization do not require a backend. If GUI access is requested, inspect the user's chosen installed stdio MCP backend and read its actual help/schema. Do not assume Cua and OCU are interchangeable or install both.

For an existing macOS CuaDriver installation, the configuration is:

```toml
[mcp.computer]
command = "/Applications/CuaDriver.app/Contents/MacOS/cua-driver"
args = ["mcp"]
```

Store it at the selected worker config path and add `--config /absolute/path/to/worker.toml` to the worker launch arguments. Restart the worker only after accounting for running tasks. Configuration changes are not hot-reloaded.

Check the backend's own daemon, Accessibility and Screen Recording permissions under the correct app identity. A permission boolean is not proof of live capture. Discover backends/tools through Macrun and test a harmless observation; if authorized, use a disposable application/workflow to verify input followed by a new observation. Do not manipulate an unrelated user's document to prove clicking works.

**Checkpoint:** discovery works, a real backend call completes, and actual screenshot content reaches the agent. If GUI is blocked, report command/file installation separately from GUI readiness.

## 7. Register the MCP frontend on the agent host

For the conventional paths above, choose installed clients:

```bash
claude mcp get macrun
# Only if absent:
claude mcp add --scope user macrun -- "$HOME/.local/bin/macrun" \
  --socket "$HOME/.local/share/macrun-server/control.sock" mcp

codex mcp get macrun
# Only if absent:
codex mcp add macrun -- "$HOME/.local/bin/macrun" \
  --socket "$HOME/.local/share/macrun-server/control.sock" mcp
```

For an existing mismatched entry, inspect and update only that entry while retaining its previous value for recovery. Do not replace the entire user config. Use the binary directly: building via `make` on MCP startup can corrupt the stdio protocol with build output.

The MCP working directory defaults to the process working directory. For a fixed project add `--workspace /absolute/server/source` before `mcp`; for several projects, pass `workspace` explicitly to `sync_start`. A fixed user-level workspace must not silently synchronize the wrong project.

**Checkpoint:** inspect the saved entry and run an actual MCP initialization, `tools/list` (14 tools in 0.2), and `device_status`. If only config inspection is possible, label it as configured but not client-tested; do not equate Codex `mcp get` with a successful live call. New agent sessions may be needed to discover the entry.

## 8. Install the usage Skill on the agent host

Copy the entire `skills/macrun` folder from the chosen revision:

- Claude Code user scope: `~/.claude/skills/macrun/`.
- Codex user scope: `~/.agents/skills/macrun/`.
- Project scope: `<project>/.claude/skills/macrun/` or `<project>/.agents/skills/macrun/`.

Inspect existing directories/symlinks first. If content already matches, do nothing; if different, compare and preserve customizations instead of silently overwriting. Verify `SKILL.md`, `references/cli.md`, and `agents/openai.yaml` arrived intact. Install on the machine where the agent runs, not only on the Mac worker.

The Skill is guidance, not an MCP registration mechanism. In a fresh session, invoke `/macrun` (Claude) or `$macrun` (Codex), or request a task matching its description. Do not claim host discovery was tested solely because the files exist.

## 9. Configure and verify a source mirror

Skip this step if no source project/destination is selected, and state that clearly. Use an isolated scratch mirror for connectivity tests; never select an existing worker directory for deletion-propagating sync without knowing it is the intended destination.

Server-side project `macrun.toml`:

```toml
remote_root = "/absolute/worker/path/to/project"
exclude = ["node_modules", "dist", ".env", ".env.*"]
sync_timeout_seconds = 120
```

Run `macrun --workspace /absolute/server/project sync` and verify a known file's contents on the worker through `file.read` or a remote command. Test a file download back to the server as a separate result-return check. If watch is requested, start `sync --watch`, edit a harmless source file, observe convergence, then either stop the test watcher or configure its requested lifecycle. Server installation alone does not create a watcher.

## 10. Completion record

Report:

- Host identities/architectures and installed Macrun revision.
- Binary, data, socket, config and service paths; no credential values.
- Service running state and configured startup behavior.
- MCP entries and Skill destinations for each selected client.
- Actual command task ID, terminal status/exit code; sync result if tested.
- Backend/GUI observation and screenshot evidence if requested and verified.
- Any untested behavior, user action needed, and how to inspect logs/restart.

Separate **configured**, **running**, and **verified**. Do not claim an LLM-driven session, successful GUI action, WAN reliability or reboot recovery from weaker checks.

## Upgrade and repeat runs

Inspect before changing state; preserve identity and existing client configuration. Wait for important worker tasks before replacing binaries/restarting. Update both ends from one compatible revision. Keep old binaries/configs available for rollback; this project has no automatic rollback installer. Do not assume backward-compatible state formats across protocol changes. Follow [operations.md](operations.md) for restart, diagnosis and 0.1 migration notes.
