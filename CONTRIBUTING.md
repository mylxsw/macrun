# Contributing to macrun

macrun is a small remote-tool runtime. Favor reusable commands, files, synchronization and MCP capabilities over project-specific compiler or application workflows.

## Work locally

1. Read [README.md](README.md), [the usage Skill](skills/macrun/SKILL.md), and the relevant source under `src/`.
2. Run `make deps` and `make check`.
3. For runtime changes, run `make smoke`. It starts isolated local server/worker processes and uses a fixture backend, not real GUI control.
4. On a Mac with Docker, `make cross-smoke` additionally checks a Linux amd64 server against the Mac worker. It is not an ARM64 or WAN test.

Use a focused branch and pull request describing the behavior before/after the change and the exact verification performed. Include a regression test when it protects an observable runtime guarantee. Do not update dependencies or reformat unrelated files as part of a narrow fix.

## Compatibility and documentation

Changes to operations, task states, retry semantics, sync behavior or protocol versions affect both ends. Update the worker/server together and document migration needs. Preserve task request-ID deduplication and avoid automatically replaying uncertain side effects.

Keep [English](README.md) and [Chinese](README.zh-CN.md) README instructions aligned. Update [agent installation](docs/agent-install.md), [operations](docs/operations.md), or [the Skill](skills/macrun/SKILL.md) when their behavior changes. Installation examples must identify which machine runs each command and use generic paths, not personal deployment addresses.

Report evidence accurately: build success, local fixture smoke tests, real remote transport, and real GUI behavior are different checks. Never publish tokens, private keys, raw private task logs, or screenshots containing unrelated personal information in issues or commits. For an issue, include OS/architecture, macrun revision, relevant redacted logs and a minimal reproduction.
