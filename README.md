# macrun

**Give a remote coding agent access to the computer where the work needs to run.**

English | [简体中文](README.zh-CN.md)

Your agent edits code on a Linux server. Your Mac builds the app, runs tests, and hosts the desktop. macrun connects them: synchronize the working copy, start commands, inspect results, and call the Mac's existing computer-use tools through MCP.

**Status: 0.2 demo · Linux/macOS · one server + one worker · MIT.** Designed for a trusted, single-user environment. macrun supplies general tools; the agent decides how to build, test, and operate your application.

[Install](#install) · [Quick start](#quick-start) · [Claude / Codex](#connect-your-agent) · [Usage](#everyday-workflows) · [Agent installation guide](docs/agent-install.md) · [Operations](docs/operations.md)

## Why macrun?

A local computer-use MCP server can operate the computer on which it runs. macrun makes that capability available to an agent on another machine, alongside commands and file synchronization.

- **Keep editing on the agent host.** Send uncommitted source changes to the worker without a Git commit or push.
- **Run long jobs without holding a tool call open.** Receive a task ID immediately, then retrieve logs, exit codes, and results.
- **Reuse local MCP tools.** Discover and call a configured stdio backend, including computer-use tools, while preserving its session and original text/image results.
- **Bring evidence back to the agent.** Read files, download artifacts, and return screenshots as MCP image content.
- **Recover from network interruptions.** Worker-owned tasks continue through connection loss. Query the same task after reconnecting instead of repeating side effects.

macrun does not replace your compiler, test framework, or desktop automation backend. There are no Xcode-specific commands: `exec_start`, files, sync, and backend calls are the building blocks.

## How it works

```text
Agent host (typically Linux)                    Worker (typically macOS)

Claude Code / Codex                             macrun worker
        │ stdio MCP                                  │
    macrun mcp → macrun serve ←── outbound QUIC ───────┤
                      ↑                              ├─ shell tasks
                 macrun CLI                          ├─ files / source mirror
                                                     └─ local stdio MCP backend
                                                        (e.g. CuaDriver)
```

The **worker initiates the connection**; it does not need a public inbound port. The server's configured UDP endpoint must be reachable, directly or over an existing private network such as Tailscale. SSH is useful for installation but is **not** macrun's runtime transport. There is no TCP fallback.

One executable provides `serve`, `worker`, CLI and `mcp` modes. Server-host paths and worker paths are different filesystems. The MCP frontend talks to the server through a local Unix socket.

## Install

The documented installation path is a source build. Git, Make, a C compiler/linker and Rust are required. `make deps` reuses an existing Rust installation or bootstraps a local toolchain, adds rustfmt/clippy when rustup is available, and fetches locked dependencies. It does not install Docker, Python, Xcode, or a computer-use backend.

- **macOS:** install Xcode Command Line Tools (`xcode-select --install`). Full Xcode is only needed by projects that require it.
- **Ubuntu/Debian:** install prerequisites with `sudo apt-get update && sudo apt-get install -y git build-essential curl ca-certificates`.
- **Docker:** needed only for the Docker-based Linux build/image targets.
- **Python 3:** needed for smoke tests, not normal server/worker operation.

### Let an agent handle setup

Give your agent this request, filling in the hosts you want to use:

> Read https://github.com/mylxsw/macrun/blob/main/docs/agent-install.md and install macrun with SERVER as the agent/server host and WORKER as the target computer. Inspect their architectures and existing configuration first. Configure my installed Claude Code/Codex clients, install the usage Skill, and verify a real remote command. Configure computer use only if a backend is available; report any remaining GUI or permission steps.

The guide is also available as [raw Markdown](https://raw.githubusercontent.com/mylxsw/macrun/main/docs/agent-install.md). Relative links resolve against this repository; when a tool only retrieves raw files, fetch the linked files from the same revision.

### Native build on each machine

Run on both the agent host and the worker, using the same source revision:

```bash
git clone https://github.com/mylxsw/macrun.git
cd macrun
# Optionally check out a chosen revision on BOTH machines before building.
make deps
make build
make install
export PATH="$HOME/.local/bin:$PATH"
macrun --version
```

`make build` writes a copy to the project's `dist/<os>-<arch>/macrun`; it does not install anything. `make install` explicitly copies the native binary to `~/.local/bin/macrun`. Use `PREFIX=/your/path` to change that installation prefix.

| Build host / target | Output |
| --- | --- |
| Apple Silicon Mac | `dist/darwin-arm64/macrun` |
| Intel Mac | `dist/darwin-amd64/macrun` |
| Linux x86-64 | `dist/linux-amd64/macrun` |
| Linux ARM64 | `dist/linux-arm64/macrun` |

### Build the Linux binary on a Mac

With Docker running, choose the **server's architecture**, not the Mac's:

```bash
make build-client                         # Native Mac binary
make build-server PLATFORM=linux/amd64     # x86-64 Linux server
# Or, for an ARM64 server:
make build-server PLATFORM=linux/arm64
```

Copy the matching `dist/linux-*/macrun` to your Linux host and install it there. These Docker-built binaries use Debian bookworm's glibc environment; they are not static musl executables. Native Linux builds do not require Docker. `make server-image` builds a runnable container image; it does not deploy one.

## Quick start

First establish a foreground connection. Add service management only after it works. `SERVER_SSH` below is your SSH alias; `SERVER_IP` is a numeric IP reachable from the worker (they need not resolve to the same interface).

### 1. Start the server — agent host

```bash
export PATH="$HOME/.local/bin:$PATH"
export MACRUN_SOCKET="$HOME/.local/share/macrun-server/control.sock"
macrun init --data "$HOME/.local/share/macrun-server"
macrun serve --listen 0.0.0.0:7443 --data "$HOME/.local/share/macrun-server"
```

Run `init` **once for a new installation**. It creates the server identity and refuses to replace an existing one. Keep `key.der` on the server. Allow UDP 7443 along the chosen network path, or bind to a reachable private interface instead of `0.0.0.0`.

### 2. Start the worker — target computer

In a worker terminal, replace the two values:

```bash
export PATH="$HOME/.local/bin:$PATH"
SERVER_SSH=your-server-ssh-alias
SERVER_IP=192.0.2.10   # Example only: replace with the reachable server IP
mkdir -p "$HOME/.local/share/macrun-worker"
scp "$SERVER_SSH:.local/share/macrun-server/cert.der" "$HOME/.local/share/macrun-worker/"
scp "$SERVER_SSH:.local/share/macrun-server/token" "$HOME/.local/share/macrun-worker/"
chmod 600 "$HOME/.local/share/macrun-worker/token"
macrun worker --server "$SERVER_IP:7443" \
  --cert "$HOME/.local/share/macrun-worker/cert.der" \
  --token-file "$HOME/.local/share/macrun-worker/token" \
  --data "$HOME/.local/share/macrun-worker"
```

The copy commands assume SSH logs in as the server's service user. No backend configuration is needed for commands, files, or sync.

### 3. Verify — a second terminal on the agent host

```bash
export PATH="$HOME/.local/bin:$PATH"
export MACRUN_SOCKET="$HOME/.local/share/macrun-server/control.sock"
macrun status
macrun exec --cwd /tmp 'hostname; uname -m; echo macrun-ok'
macrun task TASK_UUID  # Replace with the returned task_id; poll until complete
```

Expect `connected: true`, then a task with `status: succeeded`, `result.exit_code: 0`, and the **worker's** hostname plus `macrun-ok`. An accepted task is not yet a completed task.

For persistent services, restart commands and logs, continue with [Operations](docs/operations.md). For an agent performing the installation, use the [agent installation guide](docs/agent-install.md).

## Connect your agent

Run registration on the **agent/server host**, after the binary and socket are available. Inspect any existing `macrun` entry first; do not overwrite unrelated configuration.

```bash
# Claude Code
claude mcp add --scope user macrun -- "$HOME/.local/bin/macrun" \
  --socket "$HOME/.local/share/macrun-server/control.sock" mcp
claude mcp get macrun

# Codex
codex mcp add macrun -- "$HOME/.local/bin/macrun" \
  --socket "$HOME/.local/share/macrun-server/control.sock" mcp
codex mcp get macrun
```

Use a new agent session. The MCP frontend defaults to its working directory; start the agent in your source project, configure an explicit `--workspace /absolute/source/path` before `mcp`, or pass `workspace` to `sync_start`.

### Install the usage Skill

From the repository root **on the agent host**, choose the appropriate destination:

```bash
# Claude Code
mkdir -p ~/.claude/skills
test -e ~/.claude/skills/macrun || cp -R skills/macrun ~/.claude/skills/macrun

# Codex
mkdir -p ~/.agents/skills
test -e ~/.agents/skills/macrun || cp -R skills/macrun ~/.agents/skills/macrun
```

These commands leave an existing skill untouched; compare and update it deliberately when upgrading. Copy the whole folder, including references. Project-local destinations are `.claude/skills/macrun/` and `.agents/skills/macrun/`.

Invoke `/macrun` in Claude Code or `$macrun` in Codex. The [usage Skill](skills/macrun/SKILL.md) guides task execution; the [installation guide](docs/agent-install.md) guides setup. Neither installs a compiler/backend nor registers MCP automatically. See the official [Claude skills](https://code.claude.com/docs/en/skills) and [Codex skills](https://developers.openai.com/codex/skills/) documentation for host-specific discovery behavior.

Try this prompt:

> Use macrun to check the worker, synchronize this project's configured source mirror, wait for sync to succeed, run the project's tests on the worker, and report the exit code and relevant logs. If you operate the UI, discover the backend tools first and verify the result with a fresh screenshot.

## Everyday workflows

### Synchronize source, then run a command

Create `macrun.toml` in the **server-side source project**:

```toml
remote_root = "/Users/YOUR_USER/work/my-app"
exclude = ["node_modules", "dist", ".env", ".env.*"]
sync_timeout_seconds = 120
```

```bash
macrun --workspace /absolute/source/my-app sync
macrun exec --cwd /Users/YOUR_USER/work/my-app 'make test'
macrun task TASK_UUID
# Optional continuous sync; keep this process running, Ctrl-C to stop:
macrun --workspace /absolute/source/my-app sync --watch --interval-ms 1000
```

Use your project's real command and paths. Sync transfers uncommitted files, preserves executable bits and symlinks, and propagates deletion of managed files while keeping unrelated generated files. Defaults exclude `.git`, `.build`, `DerivedData`, `.macrun`, `target`. Changed files transfer whole; this is not block-level delta sync.

Sync and execution are independent. Wait for sync success before building. A failed sync may leave partial changes; it is not a transactional checkout. Each watch interval starts after the previous sync completes. To require a fixed source snapshot, pause edits/watch or use an isolated fixed copy.

### Commands, files, and artifacts

```bash
macrun exec --cwd /tmp --timeout 7200 'your-long-running-command'
macrun task TASK_UUID --offset 0
macrun cancel TASK_UUID
macrun call file.read --args '{"path":"/tmp/report.txt","text":true}'
macrun upload ./input.zip /tmp/input.zip
macrun download /tmp/report.txt ./report.txt
```

Remote paths belong to the worker; upload sources and download destinations belong to the CLI host. Logs combine stdout/stderr and use byte offsets (up to 64 KiB per read). File chunks are limited to 512 KiB; `file_image` returns existing PNG/JPEG images up to 8 MiB. Downloads detect file-version changes and may leave a `.part` file on failure; automatic resume is not implemented.

Commands have no interactive stdin/PTY. Their process group is cleaned up on completion; do not rely on `command &` to install a persistent service. Use a service manager or keep the task running.

### Computer use through a local backend

Install and validate your chosen stdio MCP backend on the worker first. For an installed CuaDriver app, create `worker.toml` on that worker:

```toml
[mcp.computer]
command = "/Applications/CuaDriver.app/Contents/MacOS/cua-driver"
args = ["mcp"]
```

Restart the worker with `--config /absolute/path/to/worker.toml` added to its existing arguments. A backend may also set `env` and `cwd`; configuration is read at worker startup. macrun does not install desktop tools or grant macOS Accessibility/Screen Recording permissions.

Discover `mcp_servers` → `mcp_tools`, save the returned backend session, submit `mcp_call` using the discovered schema, then poll `task_get`. Completed backend images are returned as image content to the agent. After a click, observe the window again: an input acknowledgement is not proof of the visual result. `file_image` reads a file; it does not take a screenshot.

## Tool contract and recovery

| MCP tools | Purpose |
| --- | --- |
| `device_status` | Connection and latest synchronization state |
| `exec_start` | Start a shell task on the worker |
| `task_get`, `task_cancel` | Read state/logs/results; request cancellation |
| `file_list`, `file_read`, `file_write`, `file_move` | Remote directory and file operations |
| `file_image` | Return an existing worker image as MCP content |
| `mcp_servers`, `mcp_tools`, `mcp_call` | Discover and call local worker backends |
| `sync_start`, `sync_get` | Start and inspect one source synchronization |

`exec_start` and `mcp_call` require a UUID `request_id`; their `task_id` is that UUID. Reuse the same ID and arguments after uncertain delivery. A new ID may repeat side effects. Sync uses a separate `job_id` queried through `sync_get`.

Task states: `accepted`, `running`, `succeeded`, `failed`, `cancelled`, `timed_out`, `unknown`. Check the state **and** command exit code/backend result. Network loss preserves worker tasks and backend sessions; worker restart marks unfinished tasks `unknown` and does not replay them. Backend timeout/cancellation/crash can invalidate its session. Rediscover tools and observe actual effects before retrying. See the [usage Skill](skills/macrun/SKILL.md) and [design notes (Chinese)](docs/design.md).

## Development and evidence

```bash
make help          # All targets and configurable defaults
make check         # Formatting, clippy, Rust tests
make smoke         # Real local transport/task/file/MCP tests; fixture GUI
make cross-smoke   # Mac host + Linux amd64 Docker server; fixture GUI
make build PROFILE=debug
```

Debug distributions use `dist/debug/<platform>/macrun`; Cargo's intermediate files remain in `target/`. `make deps` does not change shell startup files. `MACRUN_TOOLS_ROOT` overrides toolchain storage; `RUST_VERSION` selects the initial bootstrap version. See [Makefile](Makefile) for `DIST_DIR`, `TARGET_DIR`, `PREFIX`, `PLATFORM` and runtime targets.

See [validation evidence](docs/validation.md) for automated tests and the separately verified Linux ARM64 → Mac deployment with real Cua screenshots. Tests using a fixture do not establish GUI correctness. Contributions should include a focused change, relevant checks and precise evidence; see [CONTRIBUTING.md](CONTRIBUTING.md).

## Current limits

- Single server/worker; no multi-device routing or multi-user isolation.
- Tools-only MCP frontend, not a transparent proxy for resources, prompts, sampling, elicitation or progress notifications. Frontend protocol: `2025-03-26`; backend negotiation supports `2024-11-05`, `2025-03-26`, `2025-06-18`.
- No TCP fallback, interactive shell, automatic build orchestration, sync pause API or task/log retention policy.
- No claim of unattended GUI reliability through logout, sleep or reboot. Desktop access depends on the backend, OS permissions and graphical session.
- Commands and results persist on the worker. Treat this demo as trusted remote execution, not a hardened sandbox.

Upgrading from 0.1 removes specialized build/UI commands and changes the wire protocol; update both ends together. Project config now accepts only `remote_root`, `exclude`, `sync_timeout_seconds`. Worker backends use `--config`, replacing old Cua-specific flags. Use new state directories for the 0.1 → 0.2 migration and inspect existing app processes separately.

## License

[MIT](LICENSE).
