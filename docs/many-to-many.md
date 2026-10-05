# Many-to-many connections

One Linux server accepts multiple authenticated clients. One client connects to up to 16 servers simultaneously; it does not switch between them. Update both binaries to use this capability. Protocol 2 and existing single-server settings remain compatible, but an old server still accepts only one client.

## Server: choose a client

Start the existing server as usual, and connect each client with its own invitation or existing credentials. `macrun status` now returns `workers[]`, including `client_id`, instance, status/host name, heartbeat time and active synchronization. A client's UUID persists in its worker data directory across restarts. Do not clone the same data directory onto another device; duplicate client identities are refused.

```sh
macrun --socket /path/to/control.sock status
macrun --socket /path/to/control.sock --client CLIENT_UUID exec --cwd /tmp 'uname -m'
macrun --socket /path/to/control.sock --client CLIENT_UUID task RETURNED_TASK_UUID
macrun --socket /path/to/control.sock --client CLIENT_UUID --workspace /path/to/project sync
macrun --socket /path/to/control.sock --client CLIENT_UUID mcp
```

`MACRUN_CLIENT` provides the same default as `--client`. Every MCP worker tool accepts optional `client_id`; a tool's explicit argument overrides the frontend default. `device_status` lists all clients. Raw `call` operations accept routing metadata in `args.client_id`, which the server removes before forwarding to the worker.

With one online client, omission preserves the original behavior. With multiple online clients, omission returns `client_required`. An explicitly selected offline client returns `worker_offline`; selection is never redirected to another device. Task queries and cancellation must use the original client. Concurrent synchronizations to different clients have separate server journals.

## Desktop: connect several servers

In 本机 → 服务器 (or 设置 → 连接 → 服务器连接), wait for tasks to finish and disconnect before adding or removing a server. “添加服务器” redeems another server's invitation without replacing the primary connection. Connect again to start all saved connections. Removal preserves history and credentials for recovery.

Each server has its own status and reconnect loop. Tasks, details and approval prompts show their origin. The task page filters all retained history by server, with the same 50-row pagination. Workspace and backend entries retain their source even when names match. The sidebar and native menu panel show the number of online servers.

Pause, stop-all, desktop enable/disable and safety settings apply to every connection by default. Request deduplication, history and temporary approvals stay separate per server. Desktop tool calls serialize across connections on the same physical client; shell tasks can run concurrently. Directory synchronizations on one client also serialize to avoid concurrent mirror mutations. One server's outage or DNS failure does not block the others.

## CLI worker: profile manifest

Use an array of profiles with unique IDs, data directories and endpoints. The first profile's data directory holds the client's persistent identity; keep it first when editing the manifest. Use `primary` for the original profile and UUIDs for additional profiles. Keep the original worker data directory to preserve existing records.

```json
[
  {
    "id": "primary",
    "name": "Build server",
    "server": "build.example:7443",
    "cert": "/path/to/build-cert.der",
    "token_file": "/path/to/build-token",
    "data": "/path/to/worker",
    "config": "/path/to/worker.toml"
  },
  {
    "id": "541923f5-7a3b-49e9-afaa-ae06e721e44b",
    "name": "Test server",
    "server": "test.example:7443",
    "cert": "/path/to/test-cert.der",
    "token_file": "/path/to/test-token",
    "data": "/path/to/worker/servers/541923f5-7a3b-49e9-afaa-ae06e721e44b",
    "config": "/path/to/worker.toml"
  }
]
```

```sh
macrun worker --connections /path/to/connections.json --data /path/to/worker
```

Manifests contain file references, never token values. Desktop credentials remain in Keychain and are injected over the inherited pipe. Each profile validates its pinned certificate and credential independently; invalid saved configuration prevents startup with a clear error. Network/authentication failures reconnect independently.

Additional profiles namespace request IDs internally. Keep the original request UUID for retries on the same server, and use the returned task UUID for normal polling. Never assume the returned task UUID equals the original request UUID. Existing primary-connection UUIDs remain unchanged.

## Verification

```sh
make check
make build PROFILE=debug
python3 scripts/multi-smoke.py target/debug/macrun
python3 scripts/smoke.py target/debug/macrun
```

The many-to-many smoke test runs two real local QUIC servers and two clients: one client connects to both servers, and both clients connect to the first server. It verifies explicit routing, concurrent synchronization, merged/local history, source filtering, request isolation and retry deduplication, global stop, outage/DNS isolation, and stable identity across restarts. It uses disposable data and does not touch production. Native screenshots and interaction evidence are recorded in [desktop review](desktop/multi-server-review.md).
