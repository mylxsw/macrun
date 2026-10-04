<h1 align="center">Macrun</h1>

<h3 align="center">Your AI agent runs in the cloud. Now it can work on your Mac.</h3>

<p align="center">
  <a href="README.md">English</a> · <a href="README.zh-CN.md">简体中文</a> · <a href="#get-started">Get started</a> · <a href="#let-your-agent-install-it">Let your agent install it</a>
</p>

![Macrun connects a cloud AI agent to your Mac: code and commands go out; logs and screenshots come back.](docs/images/macrun-banner.png)

<p align="center"><strong>Macrun lets Claude Code, Codex, and other AI agents on a remote server run commands, sync code, and operate apps on your Mac.</strong></p>

<p align="center">Keep the agent on your Linux server. Use the Mac you already own to compile macOS apps, run tests, and interact with the desktop. Macrun sends the work to the Mac and brings back the logs, files, and screenshots the agent needs to continue.</p>

<p align="center"><strong>Open source · MIT · macOS / Linux · 0.2 demo</strong></p>

## What can I do with it?

Imagine asking an agent on your server:

> “Sync this project to my Mac, run its tests, launch the app, and check that the button works. Show me the result.”

Macrun gives the agent the tools to carry out that workflow:

| You want to… | Macrun provides… |
| --- | --- |
| Build or test code on another computer | Remote shell commands with a working directory, environment, timeout, logs, and exit code |
| Test changes before committing them | One-way source sync, including uncommitted files, plus an optional continuous watcher |
| Wait for a long build without blocking a tool call | A task ID immediately; results and progress can be queried later |
| See what happened on the Mac | File reads, artifact downloads, and screenshots returned as images to the agent |
| Click, type, and inspect desktop apps | Access to a computer-use MCP tool installed on the Mac, such as CuaDriver |

**Macrun connects the agent to the computer; your existing tools do the work.** It does not include a compiler or a desktop automation engine. For command execution and file sync, Macrun alone is enough. For desktop interaction, add a local computer-use backend. The agent chooses the project's actual build commands and verifies the results.

### Is this for me?

Macrun is useful when your agent and your execution environment are on **different machines**—for example, Claude Code on a cloud Linux server and Xcode on a Mac mini at home.

If your agent already runs on the target Mac, you may only need a local computer-use tool. If you need a full remote desktop for a human, Macrun is not that interface: it exposes tools for agents and a CLI for you.

## How it works

![Macrun architecture: Claude Code or Codex calls the Macrun MCP frontend and server on Linux. The Mac worker connects to that server and provides shell, file, and local desktop-tool access.](docs/images/macrun-architecture.png)

There are two sides:

1. **Your server:** runs the agent, holds the source code, and runs the Macrun server. The agent accesses Macrun through MCP—the tool interface supported by Claude Code and Codex.
2. **Your Mac:** runs the Macrun worker, which executes commands, receives files, and calls local desktop tools.

The Mac initiates the connection, so you do not need to open an inbound port on your home Mac. It must be able to reach the server's UDP endpoint, either directly or through an existing private network such as Tailscale. Commands and results travel over QUIC; SSH is only used in the examples to install files.

The diagram shows the common Linux → Mac setup. The executable also supports Linux workers. All modes use the same `macrun` binary.

The macOS desktop client is under full acceptance testing. See [desktop setup](desktop/README.md) and the [acceptance checklist](docs/desktop/completion-checklist.md) for build instructions, invitation pairing, safety controls and verification limits.

## Get started

The shortest path is to **connect the two machines and run one command**, then add your agent and optional desktop tools.

You need:

- A Linux server you can access, and a Mac where you can run a terminal.
- Git, Make, and a C compiler on both machines. On Mac, run `xcode-select --install`; on Ubuntu, install `git build-essential curl ca-certificates`.
- A server IP reachable from the Mac over UDP, using port `7443` below.

These instructions build from source. `make deps` prepares Rust if needed. Docker is **not required** when building on each machine. Use the same Git revision on both sides.

### 1. Install Macrun on both machines

Run this once **on the Linux server**, and again **on the Mac**:

```bash
git clone https://github.com/mylxsw/macrun.git
cd macrun
make deps
make install
export PATH="$HOME/.local/bin:$PATH"
macrun --version
```

`make install` builds and installs the executable into `~/.local/bin`. The `export` affects the current terminal; use the full binary path or add that directory to your shell's PATH for future terminals.

Prefer to build on your Mac and copy a Linux binary instead? See [Build options](#build-options).

### 2. Start the server

**On Linux**, run:

```bash
export MACRUN_SOCKET="$HOME/.local/share/macrun-server/control.sock"
macrun init --data "$HOME/.local/share/macrun-server"
macrun serve --listen 0.0.0.0:7443 \
  --data "$HOME/.local/share/macrun-server"
```

Leave this terminal running. Run `init` only for a new installation; it creates the connection credentials and refuses to overwrite an existing identity. Allow UDP `7443` on the server's chosen network path. You can bind to a private IP instead of `0.0.0.0`.

### 3. Connect your Mac

**On the Mac**, set your SSH alias and the server IP. The example IP below is a placeholder:

```bash
SERVER_SSH=your-server-ssh-alias
SERVER_IP=192.0.2.10  # Replace with your server's reachable IP
mkdir -p "$HOME/.local/share/macrun-worker"

scp "$SERVER_SSH:.local/share/macrun-server/cert.der" \
  "$HOME/.local/share/macrun-worker/"
scp "$SERVER_SSH:.local/share/macrun-server/token" \
  "$HOME/.local/share/macrun-worker/"
chmod 600 "$HOME/.local/share/macrun-worker/token"

macrun worker --server "$SERVER_IP:7443" \
  --cert "$HOME/.local/share/macrun-worker/cert.der" \
  --token-file "$HOME/.local/share/macrun-worker/token" \
  --data "$HOME/.local/share/macrun-worker"
```

Leave this terminal running too. The SSH alias must log in as the Linux user from step 2. Only the certificate and token go to the Mac; the server's `key.der` stays on Linux.

### 4. Run your first remote command

Open a **second Linux terminal**:

```bash
export PATH="$HOME/.local/bin:$PATH"
export MACRUN_SOCKET="$HOME/.local/share/macrun-server/control.sock"

macrun status
macrun exec --cwd /tmp 'hostname; uname -m; echo hello-from-mac'
macrun task TASK_UUID
```

Replace `TASK_UUID` with the `task_id` returned by `exec`. If it is still running, query it again.

**Success looks like this:** status reports `connected: true`; the task finishes with `status: succeeded` and `result.exit_code: 0`; its output contains your **Mac's hostname**, architecture, and `hello-from-mac`.

You now have a working remote connection. To keep it running after you close your terminals, follow [the background-service and restart guide](docs/operations.md).

## Give Claude Code or Codex access

Register Macrun **on the Linux server where your agent runs**. Choose your client:

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

If an entry named `macrun` already exists, inspect it first rather than creating a duplicate. Open a new agent session and ask:

> “Use Macrun to check the connected computer. Run `hostname` and `uname -m` on it, wait for the task to finish, and show me the output.”

### Add the Skill so your agent knows the workflow

MCP supplies the tools. The [Macrun Skill](skills/macrun/SKILL.md) teaches the agent when to sync, how to wait for jobs, how to recover without repeating a command, and how to verify UI actions.

From the Macrun checkout **on Linux**, install for the client you use:

```bash
# Claude Code
mkdir -p ~/.claude/skills
test -e ~/.claude/skills/macrun || cp -R skills/macrun ~/.claude/skills/macrun

# Codex
mkdir -p ~/.agents/skills
test -e ~/.agents/skills/macrun || cp -R skills/macrun ~/.agents/skills/macrun
```

Copy the whole folder. These commands leave an existing installation untouched; compare and update it when upgrading. In a new session, use `/macrun` in Claude Code or `$macrun` in Codex. You can also install per project under `.claude/skills/` or `.agents/skills/`.

## Put it to work

### Sync your project and run its tests

On **Linux**, create `macrun.toml` in the project you want to work on:

```toml
remote_root = "/Users/YOUR_USER/work/my-app"
exclude = ["node_modules", "dist", ".env", ".env.*"]
sync_timeout_seconds = 120
```

Replace `remote_root` with the intended **Mac destination**. From the Linux project directory, with `MACRUN_SOCKET` set as above:

```bash
macrun sync
macrun exec --cwd /Users/YOUR_USER/work/my-app 'make test'
macrun task TASK_UUID
```

Use your project's test command; `make test` is just an example. Sync is explicit: commands do not automatically synchronize source. Wait for sync to succeed before building. When using MCP, start the agent in the source project or pass the server-side `workspace` to `sync_start`.

To keep the working copy updated:

```bash
macrun sync --watch --interval-ms 1000
```

Keep the watcher running and stop it with Ctrl-C. It copies uncommitted changes and propagates deletion of files it previously managed, while preserving unrelated files generated on the Mac. It does not start builds. If a build requires unchanging source, pause editing and the watcher or use a fixed copy.

### Get files back

Run on **Linux**; remote paths refer to the Mac:

```bash
macrun call file.read --args '{"path":"/tmp/report.txt","text":true}'
macrun download /tmp/report.txt ./report.txt
macrun upload ./input.zip /tmp/input.zip
```

An agent can read a Mac PNG/JPEG using `file_image`. The image must already exist; this tool does not take a screenshot.

### Let the agent see and operate an app

Install a computer-use tool on **the Mac** first. Macrun connects to its stdio MCP interface. For an already installed CuaDriver app, create `worker.toml`:

```toml
[mcp.computer]
command = "/Applications/CuaDriver.app/Contents/MacOS/cua-driver"
args = ["mcp"]
```

Restart the worker from step 3 with `--config /absolute/path/to/worker.toml` appended. The backend needs its own macOS permissions and a usable graphical session; [the operations guide](docs/operations.md#computer-use-backend) explains the checks.

Then ask your agent:

> “Use Macrun to discover the computer tools, open Calculator on my Mac, calculate 12 × 34, and return a fresh screenshot confirming the result is 408.”

The agent discovers the backend's actual tools, calls them, and queries the resulting task. Screenshots come back as image content the agent can inspect. Acknowledging a click is not enough—it should read the new screen to confirm the result.

## Let your agent install it

Send this to an agent with access to your machines, replacing `SERVER` and `MAC`:

> Read https://github.com/mylxsw/macrun/blob/main/docs/agent-install.md and install Macrun with SERVER as the agent host and MAC as the worker. Inspect their architectures and existing setup, configure my installed Claude Code/Codex clients and the Macrun Skill, and verify a real remote command. If computer use is available, verify a screenshot too. Report anything that still requires my action.

[Agent installation guide](docs/agent-install.md) · [Raw Markdown for agents](https://raw.githubusercontent.com/mylxsw/macrun/main/docs/agent-install.md)

The guide covers a new install, existing configuration, service setup, client registration, and concrete acceptance checks. It does not assume your personal hostnames or install a desktop backend without checking what you use.

## What is ready—and what is not?

Macrun is a **working demo**, not a production remote-execution platform.

- **Verified:** commands, files, sync/watch, async results, and real Linux ARM64 → Mac communication; CuaDriver app launch, a Calculator button press, and a screenshot returned through MCP. See [validation evidence](docs/validation.md).
- **Connection loss:** tasks remain on the worker and can be queried after reconnecting. A worker restart can leave a task `unknown`; it is not automatically replayed. Reuse request IDs after uncertain delivery rather than creating duplicate work.
- **Current scope:** one server and one worker, trusted single-user access, QUIC/UDP only. No interactive terminal, multi-device routing, or automatic build orchestration.
- **Desktop limits:** no promise of unattended operation through sleep, logout or reboot. Actual app testing depends on your tools, permissions and login session.

Long tasks return IDs rather than blocking. The agent must check the final state and real exit code. For the full tool contract, file limits and recovery details, read [the usage Skill](skills/macrun/SKILL.md) and its [CLI reference](skills/macrun/references/cli.md).

## Build options

Build commands run at the repository root. `make build` creates project-local files; only `make install` installs the native binary.

| Command | Result |
| --- | --- |
| `make deps` | Prepare Rust/tooling and fetch locked dependencies |
| `make build-client` | Native executable: e.g. `dist/darwin-arm64/macrun` on Apple Silicon |
| `make build-server PLATFORM=linux/amd64` | Linux x86-64 executable via Docker: `dist/linux-amd64/macrun` |
| `make build-server PLATFORM=linux/arm64` | Linux ARM64 executable via Docker: `dist/linux-arm64/macrun` |
| `make install PREFIX=/your/path` | Install the native executable into `/your/path/bin/macrun` |
| `make check` | Formatting, lint and Rust tests |
| `make smoke` | Local connection, sync, command, file and MCP tests with a fixture backend |
| `make help` | All targets and configurable options |

On Linux, native `make build` needs no Docker. For cross-platform builds, choose the **Linux server's architecture**, then copy and install that binary there. Docker-built binaries use Debian bookworm/glibc, not static musl. `PROFILE=debug` writes to `dist/debug/<platform>/macrun`; intermediate build files remain in `target/`.

`make deps` does not install Xcode, Docker, Python or desktop tools. Python 3 is needed for smoke tests. On a Mac with Docker, `make cross-smoke` checks a Linux **amd64** container against the Mac worker; fixture tests are not real GUI acceptance tests.

## View operational logs

The server and worker log timestamped JSON lines to stderr by default: operation, request ID, duration and outcome, plus asynchronous task completion and MCP backend/tool names. Heartbeats and payloads are omitted. On Linux use `journalctl -u macrun-server -f`; on macOS follow the LaunchAgent’s `StandardErrorPath`. See [operational logs](docs/operations.md#operational-logs) for correlation and task status details.

## Documentation and contributing

Desktop client planning and interactive prototype (not shipped): [design documents](docs/desktop/README.md).

| Looking for… | Read… |
| --- | --- |
| An agent to install and configure Macrun | [Agent installation guide](docs/agent-install.md) |
| Background services, logs, restarts, troubleshooting and upgrades | [Operations](docs/operations.md) |
| Guidance for an agent using the tools | [Macrun Skill](skills/macrun/SKILL.md) |
| Implementation details | [Design notes (Chinese)](docs/design.md) |
| What has actually been tested | [Validation record](docs/validation.md) |
| How to contribute a fix or improvement | [Contributing](CONTRIBUTING.md) |

Found a problem? [Open an issue](https://github.com/mylxsw/macrun/issues) with your OS, architecture, Macrun revision, relevant redacted logs, and what you expected to happen.

Licensed under [MIT](LICENSE).
