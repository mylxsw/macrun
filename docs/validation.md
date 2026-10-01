# 验证记录

日期：2026-10-01。当前是可构建的 demo 实现，不是已部署到远程 Linux 服务器和家庭 Mac 的正式服务。

## 已通过

| 检查 | 证据范围 |
| --- | --- |
| Rust 单元/接口测试 | 同步变化/删除/链接/执行位、保留未管理文件、中断首次同步后的修复、类型转换失败后的修复、内容校验、取消进程组、MCP 协议与坐标换算、控制消息分片与帧大小限制 |
| `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` | 静态检查和格式，不代表运行验证 |
| macOS arm64 构建 | 本机 Rust 1.98.1，debug/release 可执行文件 |
| Linux amd64 Docker 构建 | 本机 OrbStack 的 linux/amd64 构建与运行 |
| `scripts/smoke.py` | 真实 QUIC/Unix socket：离线、增量、删除、日志、build/test 调度、busy、cancel、CLI 断开、服务端重启与 worker 重连；编译器为测试替身 |
| Counter 原生 Xcode 构建 | Xcode 27.0 (27A266a) 生成 Counter.app；示例含计数按钮、文本框、第二窗口 |
| `scripts/native-smoke.py` | 本机真实 macrun 同步 → xcodebuild → 产物检查；随后图形检查返回 no_display |
| `scripts/cross-smoke.py` | Linux amd64 容器内 server/CLI → UDP QUIC → Mac arm64 worker → 真实 xcodebuild；第二次未提交修改仅传一个源码文件并构建成功 |

测试脚本保留实际 result.json、阶段耗时和日志；本机证据位于 `.local/native-*/`、`.local/cross-*/` 和 smoke 脚本打印的临时目录。身份文件、令牌和本地构建产物不进入源代码包。

## 未验证与当前阻塞

- 当前工具执行环境的 CoreGraphics 返回 0 个可用显示器；`run` 完成构建后按约定返回 `no_display`（退出码 8）。不能据此断言物理 Mac 没有接显示器。
- 已安装 Cua Driver 0.26.0，daemon 未运行；独立 `describe` 在该执行环境发生 `NSPasteboard generalPasteboard` 空返回崩溃。未修改安装、权限或更新 Cua。
- Cua 适配通过了持久 MCP 协议夹具测试，**尚未取得真实应用的截图、点击效果或输入效果证据**。在正常图形会话启动 Cua 后，需要对实际安装版本重新执行 UI 验收。
- 未部署到真实远程 Linux 服务器和家庭网络；本地跨系统测试不证明跨国延迟、UDP 可达性或实际恢复耗时。
- 未对真实业务工程、Xcode UI 测试及 Swift Package 完成端到端验收。Swift Package 的命令路径已实现，未将其测试与 Xcode 证据混为一谈。
- 尚未做 10 次性能基线测量；没有发布 P95 或性能达标结论。

## 相对方案快照的实现调整

1. 单个 `macrun` 二进制提供 `serve`、`worker` 和普通 CLI 命令，仍是独立进程。
2. `scroll --dy` 使用 Cua 定义的有符号滚轮步数，不是 point。正数向下；范围 1–100。
3. demo 允许从图形 Terminal 启动 worker，也提供 LaunchAgent 模板；不强制“必须由 LaunchAgent 启动”作为运行门槛。启动仍检查图形用户与显示器。
4. 应用启动通过系统 `/usr/bin/open -n -g` 调用 LaunchServices，并核对实际可执行路径与进程身份；正常退出调用 NSRunningApplication。
5. Cua 使用持续的 stdio MCP 客户端代理到 daemon，按 tools/list 适配 target/delivery 字段；不固定声称已兼容 0.31.0。
6. 文档中的完整权限产品化、安全隔离、块级同步与严格 SLA 均未实现。

## 接下来如何验收 GUI

在具有显示器的登录会话中运行 CuaDriver.app，完成系统权限授权，再执行 `scripts/native-smoke.py`。取得真实 PNG 后，用 result.json 与 accessibility.json 中的真实 run/window/snapshot/token 执行点击和输入；查看计数器、文本和第二窗口变化。后台拒绝保持原错误，不自动打开前台回退。完成后在实际两台机器上重复相同流程。
