# Macrun 0.2 验证记录

更新日期：2026-10-02。区分通用远程工具自动化测试与后续真实部署，不把 fixture 当作真实 GUI 验证。

## 已通过

- `cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings`。
- `cargo test --locked`：11 项集成测试。命令环境变量/真实退出码、同 UUID 去重及冲突拒绝、结果持久化、worker 重启 unknown、超时、取消进程组、跨 pipe 分片中文、增量同步/删除/链接/执行位/中断修复/源变化检测、协议帧限制与分片。
- macOS arm64 debug 构建；Linux amd64 Docker release 构建（`macrun:generic-demo`）。
- `scripts/cross-smoke.py`：Linux amd64 容器内 server/CLI → QUIC → Mac arm64 worker。未提交源码同步成功；第二轮只传一个变化文件；命令返回 Darwin 和工作副本内容；MCP fixture 的中文与图片返回 Linux。
- `scripts/smoke.py`：真实 Unix socket 和 QUIC。持续同步；未提交改动和受管理删除；1.4 MB 随机二进制上传下载逐字节一致；文件片段读取；异步 shell 环境变量/退出码/取消/超时/去重；原始 MCP 工具结果和 PNG 内容返回；真实 stdio MCP initialize/list/call。
- 主动杀死服务器、重启：命令继续执行并能取回完整结果，本地 MCP session 和测试计数器保留。
- 主动杀死 MCP fixture：任务 unknown；旧 session 被拒绝，重新发现产生新 session。
- 主动杀死 worker、重启：未完成任务 unknown，没有自动重新执行；已完成任务记录保留。

MCP 测试使用仓库内 Python fixture，证明消息、图片和状态传递，不证明真实 GUI 自动化。

## 证据

最终本机 smoke 证据目录：`/var/folders/9h/kn7p5zbd2jbbg21h1b5mgwww0000gn/T/macrun-generic-x2vluu2o`。跨平台证据：`.local/macrun-cross-h1tqp0f6/result.json` 和 `worker.log`。这些运行数据不进入源码包。

原始测试日志保存在 `.local/generic-tests.log`；容器构建日志在 `.local/docker-generic-build.log`。smoke 脚本输出独立临时证据目录，含服务器/worker 日志、任务 result.json、output.log 和同步结果。

## 真实部署验证（2026-10-01 至 2026-10-02）

独立于以上 fixture 测试，使用 0.2 源码提交 `9bd5624` 完成了以下实际验证：

- Ubuntu 24.04 ARM64 服务端 → Apple Silicon Mac worker，通过已有 Tailscale 私网运行 QUIC。服务端 systemd active/running；Mac LaunchAgent 在图形用户登录后 running。
- 实际目录同步、watch 自动更新、命令执行（返回 Mac 主机名和 arm64）、退出码、文件读取与下载。
- Claude Code 用户级 Macrun MCP 显示 Connected；独立 stdio 协议客户端验证 14 个工具及 device_status。
- 真实 CuaDriver 0.26.0 backend 工具发现、启动计算器、点击数字 2，随后获取的新控件树与真实截图均显示 2。
- 该截图经 Macrun 的 stdio MCP `task_get` 返回 image 内容，不只是 worker 文件路径。
- 后续为 Oracle 上的 Claude/Codex 安装了提交 `3788092` 的 usage Skill，文件逐一核对一致；Codex MCP 配置已启用。没有把配置存在当作实际 Codex 模型会话验收。

部署凭据、主机地址、私人配置及原始任务日志不进入公共文档。本地部署证据位于 `.local/deployment/verification.log`、`live-discovery.txt`、`get_window_state-result.json`，这些文件不随 Git 发布。

## 尚未验证

- 没有持续的 WAN 性能 SLA 或长期重连稳定性证明；本次私网路径成功不代表任意公网 UDP 路径都可达。
- 没有完整的实际业务项目编译与 GUI 自动化验收；计算器点击不能替代项目测试。
- 没有实际 Claude/Codex 模型驱动的完整端到端会话验收；已有客户端连接状态和真实 stdio 工具协议证据。
- 没有整机重启、注销、休眠、锁屏和无头环境下的恢复验收。
- 没有多设备或多用户验证。

## 行为边界

网络断线保留任务；worker 崩溃不承诺继续执行。副作用已发生但结果未写盘时返回 unknown，不自动重试。同步失败不回滚已经安装的文件；agent 必须等待下一次完整同步成功。持续同步不会自动与命令互斥。
