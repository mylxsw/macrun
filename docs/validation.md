# macrun 0.2 验证记录

日期：2026-10-01。范围为通用远程工具重构，不把 0.1 的 Xcode/Cua 专用验证当作新版本验证。

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

## 尚未验证

- 没有部署到实际美国 Linux 和中国 Mac mini；尚未证明跨境 UDP 可达性、延迟、长时间重连稳定性。
- 没有安装或变更实际 Cua/OCU 及其 GUI 权限；截图来自 fixture，真实应用点击、输入和前后台行为仍需独立验收。
- 没有在实际 Claude Code 会话中注册 MCP；已通过真实 stdio 协议客户端验证工具发现和图片内容。
- 没有性能 SLA、多设备或多用户验证。

## 行为边界

网络断线保留任务；worker 崩溃不承诺继续执行。副作用已发生但结果未写盘时返回 unknown，不自动重试。同步失败不回滚已经安装的文件；agent 必须等待下一次完整同步成功。持续同步不会自动与命令互斥。
