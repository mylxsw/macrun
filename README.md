# macrun

在 Linux 修改代码，在 Mac 上构建、运行并观察 macOS 应用的 Rust demo。一个二进制包含 CLI、服务端和 worker 三种角色，通过 QUIC 长连接传输，窗口观察和输入交给本机 Cua Driver。

**当前验证范围见 [docs/validation.md](docs/validation.md)。构建成功、协议测试通过与真实 GUI 验收是不同的证据。**

## 已实现

- Linux/macOS 共用 Rust 协议；QUIC TLS、固定证书信任、共享令牌、心跳重连。
- Unix socket CLI；单任务、忙状态、实时日志、任务取消；CLI 退出后任务继续。
- 按内容哈希进行文件增量同步；保留执行位、内部符号链接，管理删除，记录外部链接；中断标记与下次修复。
- Xcode build/test、Swift Package build/test；固定缓存；超时和取消清理进程组。
- `.app` 路径启动、记录应用身份、停止、窗口选择；Cua 持续 MCP 连接、截图、控件树、快照绑定的点击/输入/按键/滚动。
- 每个任务的结构化结果、日志、PNG；服务端重启处理与需要人工处理的 worker 恢复状态。

## 构建与检查

需要 Rust 1.98 或更新版本；macOS 使用 Xcode 命令行工具，Linux 使用 C 编译器。

```sh
cargo build --locked --release
cargo test --locked
cargo clippy --all-targets -- -D warnings
cargo fmt --check
python3 scripts/smoke.py target/release/macrun
```

本次开发机没有预装 Rust，工具链安装在工作区 `.tools/`，未修改 shell 配置。此机器上可用 `scripts/cargo-local.sh` 替代 `cargo`；其他机器直接用标准 cargo。`scripts/smoke.py` 启动真实 server/worker 与 QUIC 连接，以编译器替身测试调度和恢复，不证明 Xcode 或 GUI 能力。

Mac 本机真实示例测试（先 `cargo build`）：

```sh
python3 scripts/native-smoke.py
```

该脚本只启动自己的 Counter 示例、server 和 worker，尝试构建、启动和 Cua 截图，结果保存在 `.local/native-*/`；结束时清理自己的进程。不会修改 Cua 安装或系统权限。

## 两台机器的配置

### Linux 服务端

在 Linux 构建或使用下文 Docker 镜像。先生成一次身份：

```sh
macrun init --data "$HOME/.local/share/macrun-server"
macrun --socket /tmp/macrun.sock serve \
  --listen 0.0.0.0:7443 \
  --data "$HOME/.local/share/macrun-server"
```

允许 Mac 连到该服务器的 UDP 7443。把 `cert.der` 与 `token` 通过现有可信通道复制到 Mac；`key.der` 留在 Linux。初始化拒绝覆盖已有身份。`serve` 保持运行，agent 另开终端调用 CLI。

### Mac worker

安装 Xcode、CuaDriver.app，保持图形用户登录，授予 Cua 辅助功能和屏幕录制权限。在正常图形会话里启动 Cua：

```sh
open -n -g -a CuaDriver --args serve
cua-driver status
cua-driver permissions status --json
macrun worker \
  --server SERVER_IP:7443 \
  --cert /absolute/path/cert.der \
  --token-file /absolute/path/token \
  --data "$HOME/.local/share/macrun-worker" \
  --cua-binary "$HOME/.local/bin/cua-driver"
```

`SERVER_IP` 替换为 IPv4 地址。demo 可先在图形 Terminal 中运行 worker；常驻安装使用 [LaunchAgent 模板](deploy/dev.macrun.worker.plist)，替换绝对路径、服务器地址和用户名。Cua 可通过 `--cua-socket` 指向已有 daemon 的明确端点。不要使用 daemon 之外的裸驱动直接持有桌面权限。

[systemd 模板](deploy/macrun-server.service) 用于 Linux。模板不会自动安装或启动，先手工验证连接。

### 项目配置

Linux 工作区根目录放 `macrun.toml`，可参考 [Counter 配置](examples/Counter/macrun.toml)。`remote_root`、`derived_data` 在 Mac 上解释；两者使用不同的固定目录。不要让 mirror 指向 Linux 源目录或日常项目工作区。

```toml
remote_root = "~/src/macrun-demo/MyApp"
derived_data = "~/Library/Caches/macrun/MyApp/DerivedData"
project = "MyApp.xcodeproj"
scheme = "MyApp"
configuration = "Debug"
app_relative_path = "Build/Products/Debug/MyApp.app"
exclude = ["node_modules", "*.xcuserstate"]
screenshot_max_edge = 1600
allow_foreground_fallback = false
```

`project`、`workspace`、`package = true` 三选一。Swift Package 用配置的 derived_data 作为 scratch 路径；普通 package 不自动生成 `.app`。Xcode 工程继承已有开发签名设置。Counter 使用本地 ad-hoc 签名，不需要开发者账号，其 scheme 仅演示 build/run；test 应对有测试 target 的实际项目或 Swift Package 使用。

同步期间暂停源码写入。默认排除 `.git`、`.build`、`DerivedData`、`.macrun`、`target`，不按 `.gitignore` 排除，也不整体排除 `.swiftpm`。`exclude` 是相对路径 glob，匹配目录后不再进入它。遇到冲突或中断时镜像可能部分更新，构建会停止；下一次 sync 扫描并修复。

## 使用

在 Linux 工作区执行：

```sh
macrun status --json
macrun sync
macrun build
macrun test --filter MyAppTests/testExample
macrun run
```

日志到 stderr，任务结束后 stdout 输出 `.macrun/artifacts/<job-id>/` 的绝对路径。读取其中的 `result.json` 获得 `run_id`、`window_id`、`snapshot_id`，从 `accessibility.json` 获取真实 element token：

```sh
macrun ui click --run RUN_ID --window WINDOW_ID --snapshot SNAPSHOT_ID --element TOKEN
macrun ui type --run RUN_ID --window WINDOW_ID --snapshot SNAPSHOT_ID --element TOKEN --text 'hello'
macrun ui click --run RUN_ID --window WINDOW_ID --snapshot SNAPSHOT_ID --x 100 --y 80
macrun ui key --run RUN_ID --window WINDOW_ID --snapshot SNAPSHOT_ID --key Return
macrun ui scroll --run RUN_ID --window WINDOW_ID --snapshot SNAPSHOT_ID --element TOKEN --dy 3
macrun shot --run RUN_ID --window WINDOW_ID
macrun stop --run RUN_ID
macrun cancel JOB_ID
```

每次动作消费快照，成功后返回新快照。失败后也先重新观察。坐标是窗口左上角起算的 point；结果提供 PNG 像素与 point 的比例，适配层再转换到 Cua 原始截图像素。**`scroll --dy` 是带符号的滚轮步数，正数向下，范围 1–100；不是 point。** 这是依照实际 Cua 接口对原设计的修正。

多窗口的普通观察必须指定 `--window`；首次 run 在结果里列出窗口并明确标记初始选择。后台拒绝默认直接返回；开启前台回退时可能影响焦点/鼠标，结果记录实际 delivery。macrun 成功只表示驱动接受操作且取得后续观察，`effect_verified=false`，业务效果由 agent 根据画面判断。

`cancel` 等待原任务清理完成并返回原任务的取消结果，退出码 12。`status` 随时可用，包含环境采样时间和上次结果。重复普通任务返回 4；不排队。

## 恢复和限制

- worker 断线停止活动构建/测试的进程组；已经完成 run 的应用可以保留，重连后快照失效。未知结果的输入动作不自动重试。
- server 重启将未完成结果标为 `server_restarted`，worker 清理后才重新注册。
- worker 崩溃可能留下 `process.json`；启动时核验进程身份。状态为 `recovery_required` 时先检查记录的 pid/进程组，确认旧任务已经结束，再把 journal 移到备份位置并重启 worker。不要直接删除记录并继续执行。
- 同步中断计划保存在 worker state 的 mirrors 子目录；下次 sync 自动修复，不需要手工移除标记。
- GUI 前提是同一登录用户、可用显示器、已运行的 Cua daemon 和系统权限。独立的 headless build/test 不依赖 Cua；UI 测试仍需要图形环境。
- 只面向可信单用户 demo，没有构建代码隔离；不承诺任意应用、后台输入、脱离进程组的子进程或严格崩溃自动恢复。
- screenshot/UI 整体超时会关闭 Cua 客户端，下次重新连接；不会自动重放动作。Cua 服务本身保持外部管理。
- 不变文件仍需扫描/哈希；大文件变化时整文件传输。协议 JSON 帧上限 16MiB，文件内容流式传输；超大型工作区清单应在后续版本分段。

## Linux amd64 镜像

```sh
docker build --platform linux/amd64 -t macrun:demo .
docker run --name macrun-demo -p 7443:7443/udp \
  -v /absolute/server-state:/state \
  -v /absolute/project:/work \
  macrun:demo --socket /tmp/macrun.sock serve --listen 0.0.0.0:7443 --data /state
docker exec macrun-demo macrun --workspace /work status --json
```

镜像含 CLI 和 server；首次身份仍需 `init`。容器内 agent/CLI 必须访问同一 Unix socket 和项目目录。本地跨系统测试不等于远程服务器到家庭网络验证。已有 `macrun:demo` 镜像时，可运行 `python3 scripts/cross-smoke.py`，使用本机 Linux 容器服务端和 Mac worker 验证真实 Xcode 构建；仅创建并清理自己的测试容器。

## 代码导航

`src/server.rs` 管理连接与任务；`src/worker.rs` 执行任务；`src/sync.rs` 处理可恢复文件同步；`src/process.rs` 清理进程组；`src/cua.rs` 适配持久 MCP；`src/model.rs` 定义两端协议；`src/main.rs` 是 CLI。

[需求快照](docs/requirements.md)和[设计快照](docs/design.md)来自本次 Obsidian 方案；实现差异与实际验证记录以 [validation.md](docs/validation.md) 为准。Cua 适配参考其[官方工作流](https://github.com/trycua/cua/blob/main/libs/cua-driver/rust/Skills/cua-driver/WORKFLOW.md)，实际兼容性以安装版本的 tools/list 与实测为准。

## License

[MIT](LICENSE) © 2026 mylxsw.
