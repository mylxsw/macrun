# macrun

让远程 AI agent 使用另一台电脑上的命令、文件和本地 MCP 工具。目标电脑主动连接服务器，无需目标电脑的公网入站端口。

典型场景：Claude Code 在美国 Linux 上修改代码，中国 Mac mini 接收工作副本、执行命令、运行桌面应用；agent 获取日志和截图，调用 computer use，自己决定后续步骤。macrun 不提供编译、Xcode 或测试框架专用工具。

**当前版本：0.2 demo。** 单服务器、单 worker；Linux/macOS；QUIC/UDP 连接。代码同步、命令执行与 MCP 调用相互独立，agent 自己组装流程。

```text
Linux：Claude Code → macrun mcp → macrun serve
                                      ⇅ QUIC
Mac：                        macrun worker（主动连接）
                               ├─ shell 后台任务
                               ├─ 文件读写和目录同步
                               └─ 持久 stdio MCP → Cua / OCU / 其他本地工具
```

## 能力

- 任意 shell 命令：工作目录、环境变量、超时、任务编号、日志字节偏移、退出码、取消进程组。
- Mac 保存任务和结果；网络断开不会取消命令或重建本地 MCP 会话。
- 用 UUID `request_id` 去重：同编号同参数返回已有任务，同编号不同参数拒绝。
- 本地 stdio MCP 服务配置、工具发现、异步调用。按 backend 串行调用，保留原始参数、文本、图片及错误结果。
- 文件列目录、分块读写、PNG/JPEG 图片内容；CLI 上传下载任意文件。
- Linux → worker 单向目录同步：包含未提交改动，文件哈希增量、受管理文件删除、符号链接和可执行位、同步中断修复；`sync --watch` 持续轮询。
- Claude Code stdio MCP 入口：14 个通用工具，不需 agent 自己解析 QUIC 协议。

## 常用任务（Makefile）

先运行 `make` 或 `make help` 查看全部命令。macrun 是同一个程序的不同运行模式，不是两套独立代码：在 Mac 构建出的程序提供客户端/worker，在 Linux 构建出的程序提供服务端/CLI。

```bash
make doctor          # 检查 Rust、C 编译器；报告 Python / Docker 状态
make deps            # 安装缺失的 Rust，准备 rustfmt/clippy，下载 Cargo.lock 依赖
make build-client    # 构建当前系统 release 程序：dist/darwin-arm64/macrun（Apple Silicon Mac）
make build-worker    # 同上，便于记忆 Mac worker 的构建入口
make build-server    # 用 Docker 构建 Linux amd64，并导出 dist/linux-amd64/macrun
make server-image    # 构建 Linux 容器镜像 macrun:generic-demo
make check           # 格式检查、clippy、Rust 测试
make smoke           # 构建后执行真实本地连接测试（GUI 使用 fixture）
make cross-smoke     # Mac 上构建两端，执行 Linux 容器 → Mac 的验证
make install         # 将当前系统程序安装到 ~/.local/bin/macrun
```

`build-client` 不会交叉编译 macOS：请在 Mac 上运行。Linux 上不使用 Docker 时，直接 `make build` 即可构建本机服务端。`build-server` 导出的程序采用 Debian bookworm 的 glibc 环境，不是静态 musl 程序；目标服务器需有兼容运行库。

构建产物统一放在当前项目的 `dist/` 下，可以直接拷贝到目标电脑，不会自动安装：

```text
dist/
├── darwin-arm64/macrun   # Apple Silicon Mac
└── linux-amd64/macrun    # Linux x86-64 服务端
```

本机构建按当前系统和架构命名，例如 Intel Mac 为 `darwin-amd64`；Linux ARM64 为 `linux-arm64`。`PROFILE=debug` 的本机产物单独放在 `dist/debug/<平台>/macrun`，避免覆盖 release。`target/` 只作为 Cargo 构建缓存保留。只有显式执行 `make install` 才会复制到 `PREFIX/bin`（默认 `~/.local/bin`）。

可覆盖参数：

```bash
make build PROFILE=debug
make build-server PLATFORM=linux/arm64
make server-image IMAGE=macrun:local
make install PREFIX=/your/install/directory
make smoke PYTHON=python3
```

`PROFILE` 只接受 `debug/release`，默认为 `release`；Docker 服务端始终构建 release。`cross-smoke` 当前只验证 Mac 宿主 + Linux amd64。其他变量见 Makefile：`DIST_DIR`、`TARGET_DIR`、`SERVER_DIST`、`DOCKER` 等。

依赖准备会优先复用当前 Rust、用户目录的 Rust，或已有工作区工具链；均不存在时，在项目 `.local/tools` 安装 Rust，不修改 shell 配置。可以通过 `MACRUN_TOOLS_ROOT` 指定存放目录；`RUST_VERSION` 指定首次安装版本。使用系统包管理器安装的 Rust 如果没有 rustup，需要自己补上 rustfmt/clippy。`make deps` 不自动安装 Docker、Python 或 computer use 应用。

系统前置环境：Mac 需要 Xcode 命令行工具（`xcode-select --install`）；Debian/Ubuntu 需要 C 编译环境（`sudo apt-get install build-essential curl ca-certificates`）。运行 smoke 需要 Python 3；构建 Linux 镜像需要启动 Docker Desktop、OrbStack 或 Docker Engine。Rust 安装来自 [官方 rustup](https://rust-lang.github.io/rustup/installation/index.html)，服务端程序通过 [Docker local exporter](https://docs.docker.com/build/exporters/local-tar/) 导出。

日常启动也有快捷入口（前台运行，Ctrl-C 停止）：

```bash
make init DATA=.local/server
make serve DATA=.local/server LISTEN=0.0.0.0:7443
make worker SERVER=SERVER_IP:7443 CERT=./cert.der TOKEN_FILE=./token WORKER_CONFIG=./worker.toml
make status
make sync WORKSPACE=/srv/code/my-app
make sync-watch WORKSPACE=/srv/code/my-app INTERVAL_MS=1000
```

`worker.toml` 可从 `examples/worker.toml` 复制后修改；不接入本地 MCP 时使用 `WORKER_CONFIG=`。`make init` 不覆盖已有身份；所有服务器调用可通过 `SOCKET` 指定 Unix socket。`make mcp` 只启动已构建的程序，不会输出构建日志；Claude Code 配置仍建议直接使用下方的二进制入口。

原始 Cargo 命令和 `scripts/cargo-local.sh` 保留可用。CI 也使用 `make check` 和 `make smoke PROFILE=debug`。

## 1. 启动服务器

在运行 Claude Code 的 Linux 上：

```bash
macrun init --data /srv/macrun
macrun --socket /tmp/macrun.sock serve --listen 0.0.0.0:7443 --data /srv/macrun
```

将生成的 `cert.der` 和 `token` 复制到目标电脑。`key.der` 留在服务器。服务器需允许所配置的 UDP 端口；当前没有 TCP/SSH 备用传输，跨境可达性和性能需要实测。

## 2. 启动目标电脑

`worker.toml` 示例：

```toml
[mcp.computer]
command = "/absolute/path/to/ocu"
args = ["mcp"]

# 也可以使用 Cua，名称完全由你指定，不必同时安装二者。
# [mcp.cua]
# command = "/absolute/path/to/cua-driver"
# args = ["mcp"]
```

每个 backend 可设置 `env = { KEY = "value" }` 和 `cwd = "/absolute/path"`。配置只在 worker 启动时读取。没有 MCP 需求时可不传 `--config`。

```bash
macrun worker --server SERVER_IP:7443 \
  --cert ./cert.der --token-file ./token \
  --data ./worker-state --config ./worker.toml
```

computer use 的安装、Mac 图形登录会话和系统权限仍由原工具负责。macrun 不会伪造显示器、AX 窗口或截屏结果。启动模板见 `deploy/dev.macrun.worker.plist`；服务端模板见 `deploy/macrun-server.service`。

## 3. 接入 Claude Code

在 Linux 的 MCP 配置中注册本地进程（路径替换为实际绝对路径）：

```json
{
  "mcpServers": {
    "macrun": {
      "command": "/usr/local/bin/macrun",
      "args": ["--socket", "/tmp/macrun.sock", "--workspace", "/srv/code/my-app", "mcp"]
    }
  }
}
```

标准输入输出只用于 MCP，诊断日志走 stderr。支持 MCP 2025-03-26；backend 兼容 2024-11-05、2025-03-26、2025-06-18。只实现工具能力，不是完整的 MCP 任意消息代理；resources、prompts、sampling/elicitation 回调和进度通知转发尚未实现。

| MCP 工具 | 作用 |
|---|---|
| `device_status` | worker 在线状态及最新同步信息 |
| `exec_start` | 提交 shell 任务，立即返回 `task_id` |
| `task_get` / `task_cancel` | 查询状态、日志、最终结果 / 请求取消 |
| `file_list` / `file_read` / `file_write` / `file_move` | 文件目录与分块操作 |
| `file_image` | 把 Mac 图片以 MCP image 内容返回 |
| `mcp_servers` / `mcp_tools` | backend 列表、原始工具定义和会话编号 |
| `mcp_call` | 异步调用 backend 工具 |
| `sync_start` / `sync_get` | 提交一次目录同步 / 查询同步完成状态 |

`exec_start`、`mcp_call` 必须传 UUID `request_id`。返回的 `task_id` 等于它，便于回复丢失后查询。同步使用独立的 `job_id`，由服务端保存结果。

## 4. 通用命令和任务

```bash
macrun exec --cwd /Users/demo/work/my-app 'pwd; make test'
macrun task TASK_UUID
macrun task TASK_UUID --offset 65536
macrun cancel TASK_UUID
```

`exec` 默认超时 3600 秒，`--timeout` 可调整，`--request-id` 可固定。`call` 是完整 JSON API，操作名称与 MCP 工具对应，使用点分隔：

```bash
macrun call exec.start --args '{"request_id":"YOUR_UUID","command":"echo $MESSAGE","cwd":"/tmp","env":{"MESSAGE":"hello"},"timeout_seconds":300}'
macrun call mcp.tools --args '{"server":"computer"}'
macrun call mcp.call --args '{"request_id":"YOUR_UUID","server":"computer","session":"SESSION_FROM_MCP_TOOLS","tool":"get_app_state","arguments":{"app":"TextEdit"}}'
```

调用者应按发现的 backend schema 组织参数，不要假定不同 computer use 产品的参数相同。`mcp.call` 默认超时 300 秒；任务结果中的 `result.result` 为 backend 的原始 MCP result。`task_get` 会把其中的图片作为图片内容交付 agent。

任务状态：`accepted`、`running`、`succeeded`、`failed`、`cancelled`、`timed_out`、`unknown`。执行命令的真实退出码在 `result.exit_code`。CLI 查询成功不等于被查询任务成功，agent 必须检查状态及退出码。stdout/stderr 合并，增量日志最多每次 64 KiB。命令 stdin 关闭，不支持交互式输入；命令结束会清理其进程组内的子进程，需要常驻的命令应保持任务运行。提交结果带有 Mac 上的 result_path，过大的原始结果可用文件接口读取。

断线后任务继续；重连后用原 `task_id` 查询。worker 重启会将未完成任务标记为 `unknown`，尝试清理记录的命令进程组，不自动重跑。MCP 超时、取消、进程崩溃时结果可能未知；该 backend 会话失效，需重新 `mcp_tools` 并重新观察界面。网络断线本身不会使 backend 会话失效。详见 [设计](docs/design.md)。

## 5. 文件和截图

```bash
macrun call file.list --args '{"path":"/tmp"}'
macrun call file.read --args '{"path":"/tmp/report.txt","offset":0,"length":65536,"text":true}'
macrun download /tmp/screenshot.png ./screenshot.png
macrun upload ./input.zip /tmp/input.zip
```

MCP 的 `file_image` 用于直接让 agent 看到 Mac 上的 PNG/JPEG（8 MiB 上限）。其他文件每次读取/写入最大 512 KiB，返回 base64 和下一字节偏移。下载比较大小及修改时间版本，发现变化则失败并保留 `.part` 文件；上传先写临时路径，再重命名。传输失败不自动续传；可重新运行。写文件不是事务目录更新。

## 6. 持续目录同步

Linux 源目录下的 `macrun.toml`：

```toml
remote_root = "~/work/my-app"
exclude = ["node_modules", "dist", ".env", ".env.*"]
sync_timeout_seconds = 120
```

```bash
macrun --workspace /srv/code/my-app sync
macrun --workspace /srv/code/my-app sync --watch --interval-ms 1000
```

`sync` 等到同步完成再返回；`sync --watch` 是需保持运行的前台进程，可交给服务管理器。间隔是每次同步完成后的等待时间，不是固定实时 SLA。断线自动重试；源文件在同步期间变化则本次失败，下一次重新扫描。

默认排除 `.git`、`.build`、`DerivedData`、`.macrun`、`target`；其他缓存或私有文件通过 exclude 配置。保留 Mac 上未受同步管理的文件。变化文件整文件传输，没有块级增量；每轮扫描都会读取文件并计算哈希。同步失败可能已经安装部分文件，只有 `succeeded` 表示本轮完整收敛。

Agent 需要自己安排同步与命令：等待一次 sync 成功后再执行；如果命令要求源码不变，应暂停 watch/停止编辑，或自行创建固定工作副本。macrun 不隐式同步，不锁定命令工作目录，也不判断是否正在编译。

## 从 0.1 迁移

这是明确的 demo 接口重构，协议版本从 1 升为 2，服务端和 worker 必须一起更新。

- 移除 `build/test/run/shot/stop/ui` 专用命令；改用 `exec`、文件和本地 MCP 调用。
- `macrun.toml` 仅保留同步参数。删除 `project/workspace/package/scheme/derived_data`、构建及 UI 参数。
- worker 的 `--cua-binary/--cua-socket` 改为通用 `--config worker.toml`。
- 推荐使用新的状态目录；旧 worker 的活动应用不会被新版本自动接管或关闭。
- 原生 Counter 工程保留为用户自己组合命令的示例，不是产品内置工作流。

## 验证和边界

具体执行证据见 [验证记录](docs/validation.md)。本地协议测试不是美中跨境部署或真实 GUI 验收。当前不支持多 worker 路由、PTY/交互式 shell 输入、完整 MCP 回调、TCP fallback、同步暂停 API 或自动任务结果清理。任务及日志保留在 worker 状态目录，敏感命令和输出也会持久化；demo 按受信任单用户环境使用。

本地 backend 的功能限制仍然存在：macrun 能传回操作结果，不会把“事件已发送”判断成“界面效果已经验证”。
