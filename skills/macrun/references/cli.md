# CLI fallback and configuration

Run these commands on the host running `macrun serve`, not automatically on the worker or the agent's unrelated local machine. If the server is reached through SSH, use the user's configured alias. Do not embed private hostnames or credentials in this skill.

## Connection and workspace

`macrun --help` and `macrun <command> --help` describe the installed version. The default socket is `/tmp/macrun.sock`; use the deployment's actual socket via `--socket` or `MACRUN_SOCKET`. Global `--workspace` defaults to the current directory, which must exist.

```bash
# Replace example paths with the existing deployment configuration.
export MACRUN_SOCKET=/path/to/control.sock
macrun status
macrun --workspace /path/to/source sync
```

The source directory contains `macrun.toml`:

```toml
remote_root = "/path/on/worker/project"
exclude = ["node_modules", "dist", ".env", ".env.*"]
sync_timeout_seconds = 120
```

`macrun sync` waits for completion; MCP `sync_start` instead returns a `job_id`. Continuous sync is a separate process:

```bash
macrun --workspace /path/to/source sync --watch --interval-ms 1000
```

Stop it with Ctrl-C when appropriate. Do not launch duplicate watchers for the same mirror.

## Commands, files and backend discovery

```bash
# Generate a UUID, record it, then substitute it below.
macrun exec --request-id UUID --cwd /path/on/worker/project --timeout 3600 'pwd; uname -m'
macrun task UUID
macrun task UUID --offset 65536
macrun cancel UUID

macrun call file.read --args '{"path":"/path/on/worker/report.txt","offset":0,"length":65536,"text":true}'
macrun upload ./local-input.zip /path/on/worker/input.zip
macrun download /path/on/worker/report.txt ./local-report.txt

macrun call mcp.servers
macrun call mcp.tools --args '{"server":"BACKEND_NAME"}'
```

Upload sources and download destinations are server-host paths; remote paths belong to the worker. CLI raw backend output is JSON and may contain base64. Prefer the MCP frontend for displaying images to a multimodal agent.

A backend call, after discovering its session and schema:

```bash
macrun call mcp.call --args '{"request_id":"UUID","server":"BACKEND_NAME","session":"DISCOVERED_SESSION","tool":"DISCOVERED_TOOL","arguments":{}}'
macrun task UUID
```

CLI generic operations use dotted names (`exec.start`, `task.get`, `mcp.tools`); MCP tool names use underscores (`exec_start`, `task_get`, `mcp_tools`). For sync and status prefer the named CLI commands rather than guessing a dotted operation from the MCP name.

## MCP registration shape

A stdio entry should execute the installed binary directly, with the actual socket. Include `--workspace /path/to/source` when the host does not start the MCP process in the intended project. Do not invoke a build command as the MCP entrypoint: build output would pollute the protocol stream.

```json
{
  "mcpServers": {
    "macrun": {
      "command": "/absolute/path/to/macrun",
      "args": ["--socket", "/path/to/control.sock", "--workspace", "/path/to/source", "mcp"]
    }
  }
}
```

This is an entry example, not a file to overwrite existing user configuration. Skill installation and MCP registration are separate: loading the skill does not install or start macrun. If tools are unavailable, check the installed binary, frontend configuration, socket and worker status before changing deployment.
