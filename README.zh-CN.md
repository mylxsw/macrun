# macrun

**让远程编码 Agent 使用真正需要执行工作的那台电脑。**

[English](README.md) | 简体中文

Agent 在 Linux 服务器上编辑代码，Mac 负责构建应用、运行测试和桌面操作。macrun 将两端连接起来：同步工作副本、提交命令、查询结果，并通过 MCP 调用 Mac 上已有的 computer-use 工具。

**当前状态：0.2 demo · Linux/macOS · 单服务器 + 单 worker · MIT。** 面向受信任的单用户环境，提供通用能力，由 Agent 自己决定如何编译、测试和操作应用。

[安装](#安装) · [快速开始](#快速开始) · [Claude / Codex](#接入-agent) · [日常使用](#日常使用) · [Agent 安装手册（英文）](docs/agent-install.md) · [运维手册（英文）](docs/operations.md)

## 为什么需要 macrun

本地 computer-use MCP 服务通常操作它所在的电脑。macrun 让另一台机器上的 Agent 也能调用这些能力，同时提供命令执行和文件同步。

- **保留服务器上的开发流程。** 未提交的源码也能同步，不需要先 Git commit/push。
- **长任务异步执行。** 工具立即返回任务编号，之后查询日志、退出码和结果。
- **复用已有本地 MCP 工具。** 发现并调用 stdio backend，保留会话和原始文本、图片结果。
- **把证据传给 Agent。** 读取文件、下载产物，以 MCP image 内容返回截图。
- **网络中断后继续查询。** worker 持有任务，断线不会自动取消；恢复后查询原任务，避免重复副作用。

macrun 不替代编译器、测试框架或桌面自动化工具，也没有专门的 Xcode 命令。Agent 通过 shell、文件、同步和 backend 调用组合工作流。

## 工作方式

```text
Agent 主机（通常为 Linux）                     worker（通常为 macOS）

Claude Code / Codex                            macrun worker
        │ stdio MCP                                  │
    macrun mcp → macrun serve ←── 主动拨出的 QUIC ─────┤
                      ↑                              ├─ shell 任务
                 macrun CLI                          ├─ 文件 / 源码镜像
                                                     └─ 本地 stdio MCP backend
                                                        （例如 CuaDriver）
```

**worker 主动连接服务器**，目标电脑不需要公网入站端口。服务器配置的 UDP 地址必须可达，可以使用公网，也可以使用已有的 Tailscale 等私网。SSH 可用于部署，但不是 macrun 的运行时传输；当前没有 TCP fallback。

同一个程序提供 `serve`、`worker`、CLI 和 `mcp` 模式。Agent 主机路径与 worker 路径属于不同文件系统，MCP 前端通过本机 Unix socket 访问服务端。

## 安装

本文提供从源码构建的安装方式，需要 Git、Make、C 编译器/链接器及 Rust。`make deps` 复用已有 Rust，缺失时安装本地工具链；有 rustup 时补齐 rustfmt/clippy，并下载锁定依赖。它不会安装 Docker、Python、Xcode 或 computer-use backend。

- **macOS：** `xcode-select --install` 安装命令行工具。只有待构建项目要求时才需要完整 Xcode。
- **Ubuntu/Debian：** `sudo apt-get update && sudo apt-get install -y git build-essential curl ca-certificates`。
- **Docker：** 仅 Docker Linux 构建/镜像目标需要。
- **Python 3：** 冒烟测试需要，普通 server/worker 运行不需要。

### 让 Agent 协助安装

把下面的请求发给 Agent，并替换主机信息：

> 阅读 https://github.com/mylxsw/macrun/blob/main/docs/agent-install.md ，将 macrun 的服务端安装到 SERVER，worker 安装到 WORKER。先检查两端架构和已有配置，再接入已安装的 Claude Code/Codex、安装使用 Skill，并验证真实远程命令。有可用 backend 时再配置 computer use，明确报告尚需完成的 GUI 或授权步骤。

也可直接读取 [原始 Markdown](https://raw.githubusercontent.com/mylxsw/macrun/main/docs/agent-install.md)。相对链接以本仓库为基准，仅支持原始文件读取的工具应继续获取同一版本的关联文件。

### 在两台机器各自构建

在 Agent 主机和 worker 上分别运行，并确保采用同一源码版本：

```bash
git clone https://github.com/mylxsw/macrun.git
cd macrun
# 如需固定版本，在两端构建前 checkout 相同提交。
make deps
make build
make install
export PATH="$HOME/.local/bin:$PATH"
macrun --version
```

`make build` 仅把产物复制到项目内的 `dist/<系统>-<架构>/macrun`，不会安装。显式执行 `make install` 才安装到 `~/.local/bin/macrun`，可用 `PREFIX=/your/path` 改变安装前缀。

| 构建主机 / 目标 | 产物 |
| --- | --- |
| Apple Silicon Mac | `dist/darwin-arm64/macrun` |
| Intel Mac | `dist/darwin-amd64/macrun` |
| Linux x86-64 | `dist/linux-amd64/macrun` |
| Linux ARM64 | `dist/linux-arm64/macrun` |

### 在 Mac 上构建 Linux 程序

启动 Docker，根据 **服务器架构** 选择目标：

```bash
make build-client                         # 本机 Mac 程序
make build-server PLATFORM=linux/amd64     # x86-64 Linux
# ARM64 Linux 则使用：
make build-server PLATFORM=linux/arm64
```

将对应的 `dist/linux-*/macrun` 拷贝到服务器安装。Docker 产物使用 Debian bookworm 的 glibc 环境，不是静态 musl 程序。Linux 本机构建不需要 Docker。`make server-image` 只构建可运行的容器镜像，不自动部署。

## 快速开始

先以前台进程验证连接，再配置常驻服务。下文 `SERVER_SSH` 是 SSH 别名，`SERVER_IP` 是 worker 可以访问的服务器数字 IP，两者可以走不同网络接口。

### 1. 启动服务端——Agent 主机

```bash
export PATH="$HOME/.local/bin:$PATH"
export MACRUN_SOCKET="$HOME/.local/share/macrun-server/control.sock"
macrun init --data "$HOME/.local/share/macrun-server"
macrun serve --listen 0.0.0.0:7443 --data "$HOME/.local/share/macrun-server"
```

新部署只运行一次 `init`，它生成服务器身份且拒绝覆盖已有身份。`key.der` 留在服务器。确保网络路径允许 UDP 7443，也可以用可达的私网 IP 替换 `0.0.0.0` 以限制监听接口。

### 2. 启动 worker——目标电脑

在 worker 终端替换两个变量值：

```bash
export PATH="$HOME/.local/bin:$PATH"
SERVER_SSH=your-server-ssh-alias
SERVER_IP=192.0.2.10   # 仅为示例，请替换为可达服务器 IP
mkdir -p "$HOME/.local/share/macrun-worker"
scp "$SERVER_SSH:.local/share/macrun-server/cert.der" "$HOME/.local/share/macrun-worker/"
scp "$SERVER_SSH:.local/share/macrun-server/token" "$HOME/.local/share/macrun-worker/"
chmod 600 "$HOME/.local/share/macrun-worker/token"
macrun worker --server "$SERVER_IP:7443" \
  --cert "$HOME/.local/share/macrun-worker/cert.der" \
  --token-file "$HOME/.local/share/macrun-worker/token" \
  --data "$HOME/.local/share/macrun-worker"
```

以上复制命令假定 SSH 登录用户就是 server 运行用户。仅使用命令、文件和同步时不需要 backend 配置。

### 3. 验证——Agent 主机的第二个终端

```bash
export PATH="$HOME/.local/bin:$PATH"
export MACRUN_SOCKET="$HOME/.local/share/macrun-server/control.sock"
macrun status
macrun exec --cwd /tmp 'hostname; uname -m; echo macrun-ok'
macrun task TASK_UUID  # 替换为返回的 task_id；未完成则稍后再查
```

预期连接状态为 `connected: true`；任务最终为 `succeeded`、`result.exit_code: 0`，输出包含 **worker 的** 主机名及 `macrun-ok`。任务已受理不代表已完成。

常驻运行、重启与日志见 [运维手册](docs/operations.md)。让 Agent 代为安装时使用 [Agent 安装手册](docs/agent-install.md)。

## 接入 Agent

以下注册命令在 **Agent/server 主机** 上执行，前提是程序和 socket 已就绪。先检查已有的 `macrun` 配置，不覆盖其他条目。

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

新开 Agent 会话使用。MCP 默认采用启动时的工作目录：从源码项目启动 Agent，或在 `mcp` 参数前添加 `--workspace /absolute/source/path`，也可以给 `sync_start` 显式传 `workspace`。

### 安装使用 Skill

在 **Agent 主机** 的本仓库根目录，选择对应目录：

```bash
# Claude Code
mkdir -p ~/.claude/skills
test -e ~/.claude/skills/macrun || cp -R skills/macrun ~/.claude/skills/macrun

# Codex
mkdir -p ~/.agents/skills
test -e ~/.agents/skills/macrun || cp -R skills/macrun ~/.agents/skills/macrun
```

命令保留已存在的 Skill；升级时先比较再更新。需要复制整个目录，包括 references。也可放到项目的 `.claude/skills/macrun/` 或 `.agents/skills/macrun/`。

Claude Code 用 `/macrun`，Codex 用 `$macrun`。[使用 Skill](skills/macrun/SKILL.md) 负责日常任务指导，[安装手册](docs/agent-install.md) 负责部署配置；它们不会自动安装编译器/backend 或注册 MCP。发现机制参考 [Claude 官方文档](https://code.claude.com/docs/en/skills) 和 [Codex 官方文档](https://developers.openai.com/codex/skills/)。

示例提示词：

> 使用 macrun 检查 worker，同步当前项目配置的源码镜像，等待成功后在 worker 上运行项目测试，报告退出码和相关日志。需要界面操作时先发现 backend 工具，操作后获取新的截图验证效果。

## 日常使用

### 同步后执行

在 **server 端源码项目** 中创建 `macrun.toml`：

```toml
remote_root = "/Users/YOUR_USER/work/my-app"
exclude = ["node_modules", "dist", ".env", ".env.*"]
sync_timeout_seconds = 120
```

```bash
macrun --workspace /absolute/source/my-app sync
macrun exec --cwd /Users/YOUR_USER/work/my-app 'make test'
macrun task TASK_UUID
# 可选的持续同步，保持此进程运行，Ctrl-C 停止：
macrun --workspace /absolute/source/my-app sync --watch --interval-ms 1000
```

路径和命令按实际项目替换。同步包含未提交文件，保留执行位和符号链接，传播受管理文件的删除，保留无关生成文件。默认排除 `.git`、`.build`、`DerivedData`、`.macrun`、`target`。变化文件整文件传输，不是块级增量同步。

同步和执行互相独立。构建前等待同步成功，失败可能留下部分变化，不是事务式 checkout。watch 的间隔从上一轮完成后开始计算；要求固定源码时，暂停编辑和 watch，或使用固定副本。

### 命令、文件与产物

```bash
macrun exec --cwd /tmp --timeout 7200 'your-long-running-command'
macrun task TASK_UUID --offset 0
macrun cancel TASK_UUID
macrun call file.read --args '{"path":"/tmp/report.txt","text":true}'
macrun upload ./input.zip /tmp/input.zip
macrun download /tmp/report.txt ./report.txt
```

远程路径属于 worker，上传源和下载目标属于 CLI 主机。stdout/stderr 合并，以字节偏移读取，每次最多 64 KiB。文件每块最多 512 KiB；`file_image` 返回最多 8 MiB 的现有 PNG/JPEG。下载检测文件版本变化，失败可能保留 `.part`，没有自动断点续传。

命令没有交互式 stdin/PTY，结束时清理进程组。不要依赖 `command &` 安装常驻服务，应使用服务管理器或保持任务运行。

### 接入 Computer Use

先在 worker 上安装并验证选定的 stdio MCP backend。对于已安装的 CuaDriver，在 worker 创建 `worker.toml`：

```toml
[mcp.computer]
command = "/Applications/CuaDriver.app/Contents/MacOS/cua-driver"
args = ["mcp"]
```

在现有 worker 启动参数上追加 `--config /absolute/path/to/worker.toml`，然后重启 worker。backend 也可配置 `env`、`cwd`；配置仅在 worker 启动时读取。macrun 不安装桌面工具，也不自动授予 macOS 辅助功能/屏幕录制权限。

先调用 `mcp_servers` → `mcp_tools`，保存 backend session，按发现的 schema 提交 `mcp_call`，再轮询 `task_get`。完成的图片结果直接作为 image 内容返回 Agent。点击后再次观察窗口，输入事件成功不等于界面效果正确。`file_image` 只读取已有图片，不负责截图。

## 工具与恢复规则

| MCP 工具 | 用途 |
| --- | --- |
| `device_status` | 连接与最近一次同步状态 |
| `exec_start` | 在 worker 提交 shell 任务 |
| `task_get`、`task_cancel` | 查询状态/日志/结果，请求取消 |
| `file_list`、`file_read`、`file_write`、`file_move` | 远程目录和文件操作 |
| `file_image` | 将现有 worker 图片作为 MCP 内容返回 |
| `mcp_servers`、`mcp_tools`、`mcp_call` | 发现并调用 worker 本地 backend |
| `sync_start`、`sync_get` | 提交和查询一次源码同步 |

`exec_start`、`mcp_call` 要求 UUID `request_id`，它就是返回的 `task_id`。发送结果不确定时复用同一编号和参数；换新编号可能重复执行。同步使用独立的 `job_id`，通过 `sync_get` 查询。

任务状态包括 `accepted`、`running`、`succeeded`、`failed`、`cancelled`、`timed_out`、`unknown`。检查状态及命令退出码/backend 结果。断线保留 worker 任务和 backend 会话；worker 重启把未完成任务标为 `unknown`，不自动重跑。backend 超时、取消、崩溃可能使会话失效，重试前重新发现工具并观察实际效果。详见 [使用 Skill](skills/macrun/SKILL.md) 和 [设计说明](docs/design.md)。

## 开发与验证

```bash
make help          # 所有目标与默认参数
make check         # 格式检查、clippy、Rust 测试
make smoke         # 真实本机传输/任务/文件/MCP，GUI 使用 fixture
make cross-smoke   # Mac + Linux amd64 Docker server，GUI 使用 fixture
make build PROFILE=debug
```

debug 产物在 `dist/debug/<平台>/macrun`，Cargo 中间文件保留在 `target/`。`make deps` 不修改 shell 启动文件；`MACRUN_TOOLS_ROOT` 可指定工具链目录，`RUST_VERSION` 指定首次安装版本。`DIST_DIR`、`TARGET_DIR`、`PREFIX`、`PLATFORM` 及运行快捷命令见 [Makefile](Makefile)。

[验证记录](docs/validation.md) 区分自动化测试与真实 Linux ARM64 → Mac 部署及 Cua 截图验证。fixture 测试不证明真实 GUI 正确。贡献应包含聚焦的改动、相关检查和准确证据，详见 [贡献说明（英文）](CONTRIBUTING.md)。

## 当前限制

- 单服务器、单 worker；没有多设备路由或多用户隔离。
- MCP 只提供工具，不透明代理 resources、prompts、sampling、elicitation 或进度通知。前端协议为 `2025-03-26`，backend 支持 `2024-11-05`、`2025-03-26`、`2025-06-18`。
- 无 TCP fallback、交互式 shell、自动构建编排、同步暂停 API 或任务/日志保留策略。
- 不承诺注销、休眠或重启后的无人值守 GUI 稳定性；桌面访问取决于 backend、系统权限和图形会话。
- 命令与结果保存在 worker，本 demo 是受信任环境下的远程执行工具，不是强化隔离沙箱。

从 0.1 升级会移除专用构建/UI 命令并改变协议，两端需要一起升级。项目配置现在只接受 `remote_root`、`exclude`、`sync_timeout_seconds`；worker 用 `--config` 配置 backend，替代旧 Cua 参数。0.1 → 0.2 迁移推荐使用新状态目录，单独检查原有应用进程。

## 许可证

[MIT](LICENSE)。
