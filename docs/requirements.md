> [!summary] Demo 目标
> macrun 让远程 Linux 上的编码 agent 把尚未提交的代码传到家里的 Mac mini，完成构建、测试、启动、窗口观察和界面操作。当前优先验证这条完整流程是否可用，不以产品级安全、极限性能或全面兼容作为 demo 门槛。

相关实现见 [设计文档](design.md)。更新日期：2026-10-01。本次调整依据文档评估与用户确认的 demo 优先原则；下列行为是待实现、待验收的要求，不代表已经验证通过。

## 目标与范围

源码唯一来源是 Linux 工作区，包含未提交文件。Mac 位于家庭网络，只主动连接 Linux 服务端。保留 Rust、QUIC 和 Cua Driver 的技术选择；先支持一台服务器、一台 Mac、一个配置好的工作区和一个被测应用运行会话。

Demo 要回答三个问题：真实跨国链路能否稳定传输；真实项目能否在 Mac 上构建并启动；agent 能否看见窗口、操作控件并依据新画面继续修改代码。

先选一个已有的 macOS Xcode 工程验收。Swift Package 的 build/test 在主流程通过后补充，不阻塞第一轮 demo。多机、队列、通用远程桌面、图标识别、录屏和发布签名公证不在本轮。

## 运行形态与前提

- Linux：常驻 `macrun-server`，CLI `macrun` 通过本机 Unix socket 提交命令。
- Mac：`macrun-worker` 作为已登录图形用户的 LaunchAgent 运行，主动建立并复用 QUIC 连接。
- Cua：安装 `CuaDriver.app`，手工授予辅助功能和屏幕录制权限，启动常驻 daemon；worker 使用本机接口连接。
- Mac 已安装可构建目标项目的 Xcode。用户保持登录且桌面可操作；demo 可手工登录，无需实现自动登录。无显示器时先使用 HDMI 仿真插头。
- 本轮信任 Linux、Mac、操作人员和传入代码。安全隔离、权限收缩、配对管理与凭据轮换留到产品化。结构化构建参数用于稳定调用，不宣称能隔离构建脚本、测试或被测应用。
- 首次安装手工配置服务器地址、服务器证书信任与共享连接令牌；使用 QUIC 库现成的 TLS 能力，不自研一次性配对与双向身份管理。

## 任务与运行会话

`job_id` 标识一次命令执行；`run_id` 标识一次成功启动的应用实例。`run` 命令结束后应用可以继续运行，后续 `shot/ui/stop` 引用该运行会话。构建任务结束与应用退出是两件事。

| 命令 | 行为 |
| --- | --- |
| `sync` | 仅同步源码 |
| `build` | 同步后执行 `xcodebuild build` |
| `test` | 同步后执行 `xcodebuild test`，支持测试标识过滤 |
| `run` | 同步、构建、定位产物、启动、等待窗口、返回观察 |
| `shot` / `ui snapshot` | 获取当前应用指定窗口的截图与控件树，不同步 |
| `ui click/type/key/scroll` | 对指定快照和窗口执行动作，再获取新观察 |
| `stop` | 退出记录的应用，等待 5 秒，必要时终止；不用于取消构建 |
| `cancel <job-id>` | 取消当前任务，等待 worker 清理完成 |
| `status` | 返回连接、当前任务、运行会话及最近的环境检查信息 |

同时只执行一个普通任务，第二个立即返回 `busy` 和当前 job id，不排队。`status`、`cancel` 不受普通任务占用限制。CLI 退出不取消任务；可用 `status` 找到 job id 和结果位置，再显式取消。

## 连接与恢复

双方每 5 秒发送一次心跳，连续 15 秒未收到对端数据视为断开。重连退避为 1、2、4、5 秒，之后最多间隔 5 秒，单次连接尝试也设超时。不承诺“网络恢复后 5 秒上线”；记录从恢复网络到重新就绪的实际耗时。

断线使未结束任务失败，`error.code = worker_disconnected`。worker 检测到断线后停止该任务的构建、测试等进程组并清理，不自动重放任务或界面动作。新连接先报告本地清理状态，清理完成之前不接受新任务。

server 重启后将未完成任务标记为 `server_restarted`；worker 重启后报告 `worker_restarted`，检查遗留任务与镜像状态。无法确认遗留进程已经结束时返回 `recovery_required`，由操作人员处理，不直接接新任务。已完成 run 的应用可以保留，但重新连接后必须核验实例并重新观察；旧快照一律失效。

## 源码同步

1. Linux 扫描工作区，按内容哈希识别新增、变化与删除。demo 对变化文件整文件传输，不实现滚动校验、块级差量或内容仓库。
2. 默认排除 `.git`、`.build`、`DerivedData`、`.macrun`、`target`。不整体排除 `.swiftpm`，其中可能存在共享项目配置；需要排除的本地缓存由项目追加配置。不直接套用 `.gitignore`。
3. 保留文件可执行位。内部相对符号链接可以复制；内部绝对链接改写为指向镜像内目标的相对链接。外部链接跳过并记录。遇到目标文件系统的大小写冲突，明确报错。
4. 同步只删除旧同步清单管理、而本次不存在或被排除的路径，不清理镜像内其他生成文件。文件与目录类型转换应纳入同步处理。
5. 开始修改镜像前持久化 `sync_incomplete` 标记。文件先写临时路径并校验哈希，再逐个替换；所有更新与删除完成后写入新清单，最后移除标记。
6. 不承诺同步失败后整目录回滚。失败时镜像可能部分更新，但禁止构建和测试；下次同步重新核验镜像实际内容并修复，不能仅相信旧清单。修复成功才恢复可构建状态。
7. demo 同步期间约定 agent 暂停写入工作区。检测到扫描前后或读文件期间发生变化时，返回 `source_changed`，重新同步；不宣称提供持续编辑下的原子工作区快照。
8. 构建缓存使用 Mac 上固定路径，不同步回 Linux。结果目录 `.macrun/artifacts` 也不传到 Mac。

默认同步超时 2 分钟，可为首次传输调大。验收目标是“不变文件不重传、删除生效、中断后可恢复”。大文件仅改几个字节仍可能整文件发送，这是本轮接受的限制。

## 构建、启动与停止

Xcode 使用配置的 project 或 workspace、scheme、configuration，destination 为 `platform=macOS`，并指定固定 DerivedData 路径。参数以数组传递，不经过 shell 拼接。沿用项目已有的本地开发签名设置，不默认强制 `CODE_SIGNING_ALLOWED=NO`。

Demo 显式配置待启动 `.app` 相对于 DerivedData 的路径，避免猜测 scheme 的多个产物。构建成功后确认该路径存在且可启动，通过绝对路径启动，并核验实际应用路径、bundle id、pid 与进程启动时间。发现同 bundle id 的其他实例导致无法确认启动目标时，返回 `app_instance_conflict`，不复用未知实例。

新的 `run` 在构建成功并确认产物之后才停止旧运行会话，再启动新实例。构建失败时保留旧应用；不承诺旧应用正在使用的构建产物完全不受重新构建影响。启动失败也不自动恢复旧实例。首次 demo 不覆盖应用自更新、复杂辅助进程或特殊单实例启动协议。

启动后分配 `run_id`。应用已启动但截图失败时，保留 run id、构建结果和 pid，任务失败；修复 Cua 后可以对同一 run 重试观察。应用提前退出返回 `app_exited`，超过等待时间仍无窗口返回 `window_not_found`。

默认构建和测试超时 15 分钟，启动等待窗口 60 秒，单次观察或界面动作 30 秒，均可配置。取消与超时需要停止任务进程组，不只结束顶层 `xcodebuild`。清理未完成时保持不可接任务。

`stop` 校验记录的进程启动时间与应用路径，避免把被复用的 pid 当作原应用。stop 成功后运行会话及其快照失效。

## 窗口观察与操作

每次观察返回 `run_id`、`window_id`、`snapshot_id`、窗口标题、截图和控件树。截图长边默认不超过 1600 像素，同时返回原始窗口 point 尺寸、截图像素尺寸、坐标原点和换算比例。

只有一个窗口时可自动选择；多个窗口时先返回窗口列表，要求 `--window` 明确选择。`run` 可成功返回窗口列表及其中一个窗口的初始观察，但必须写明选中的 window id。每次只捕获一个明确窗口，取消“默认抓取前五个”的要求。

所有动作绑定同一 run、window 和 snapshot。元素 token 由 macrun 映射到该次 Cua 观察；坐标以该窗口左上角为原点、单位为 point。对同一窗口再次观察使旧快照失效；动作消费该快照，失败后也需重新观察再操作。窗口尺寸变化或目标不匹配时拒绝动作。快照不能冻结应用自身变化，agent 仍需依据最新画面判断。

默认后台投递。仅在配置 `allow_foreground_fallback = true` 且 Cua 明确返回 `background_unavailable` 时，允许重试一次前台投递。前台可能改变焦点或鼠标位置，结果必须记录 `delivery`；未知错误、断线或结果丢失不自动重试，避免重复点击或输入。

动作成功定义为：驱动接受动作，且动作后的新截图与控件树已取得。是否达到业务目的由 agent 判断，macrun 记录 `effect_verified = false`，不把“按钮已点击”写成“业务操作已完成”。动作已接受但观察失败时，任务失败，并保留 `action_status = accepted`，指导调用者先重新观察。

Cua 缺失或权限不足只阻止观察与输入，不阻止纯构建和非 UI 测试；依赖 GUI 的 UI 测试仍需要相应图形环境。游戏、Metal、自绘复杂控件和安全输入框不作为 demo 验收对象。

## 命令与输出

```text
macrun serve
macrun status --json
macrun sync
macrun build
macrun test
macrun run
macrun shot --run <run-id> --window <window-id>
macrun ui snapshot --run <run-id> --window <window-id>
macrun ui click --run <run-id> --window <window-id> --snapshot <snapshot-id> --element <token>
macrun ui click --run <run-id> --window <window-id> --snapshot <snapshot-id> --x <point> --y <point>
macrun ui type --run <run-id> --window <window-id> --snapshot <snapshot-id> --element <token> --text <文字>
macrun ui key --run <run-id> --window <window-id> --snapshot <snapshot-id> --key <键名>
macrun ui scroll --run <run-id> --window <window-id> --snapshot <snapshot-id> --element <token> --dy <point>
macrun stop --run <run-id>
macrun cancel <job-id>
```

任务日志持续打印到 stderr；结束时 stdout 最后一行为结果目录。`status --json` 直接输出 JSON，不创建任务。cancel 在目标任务完成清理后返回其结果位置；取消后的原任务以 `cancelled` 结束。

```text
.macrun/artifacts/<job-id>/
  result.json
  build.log
  test.log
  run.log
  accessibility.json
  screenshots/001.png
```

仅生成实际产生的文件。`result.json` 包含 `job_id`、`kind`、`status`、`exit_code`、开始结束时间、阶段耗时、同步文件数与传输字节数、工程配置、产物路径、日志路径。涉及运行会话时包含 run id、bundle id、pid、进程启动时间；涉及观察时包含窗口列表、选中 window id、snapshot id 和截图坐标信息。

动作增加 `action_status`（`not_sent/accepted/refused/unknown`）、`observation_status`（`not_attempted/succeeded/failed`）、`effect_verified`、`delivery`（`background/foreground/unknown`）及 Cua 原始拒绝码。断线导致无法确认是否执行时使用 `unknown`。失败包含 `error: {code, message}`。

| 退出码 | 错误类别与主要 error.code |
| --- | --- |
| 0 | 成功 |
| 1 | `build_failed`、`test_failed` |
| 2 | 用法或配置错误：`invalid_config`、`artifact_not_found` |
| 3 | `worker_offline`、`worker_disconnected` |
| 4 | `busy` |
| 5 | `sync_failed`、`source_changed`、`path_conflict` |
| 6 | `timed_out` |
| 7 | `connection_auth_failed` |
| 8 | `no_gui_session`、`no_display`、`not_gui_agent` |
| 9 | `window_not_found`、`window_required` |
| 10 | `cua_unavailable`、`cua_refused`、`capture_permission_denied`、`observation_failed` |
| 11 | `app_not_running`、`app_exited`、`app_instance_conflict`、`target_mismatch`、`stale_snapshot` |
| 12 | `cancelled` |
| 13 | `server_restarted`、`worker_restarted`、`recovery_required` |

任务一经分配 job id 就有结果目录，即使失败也保留已有日志。多个具体错误可属于同一个退出码，agent 优先读取 error.code。服务离线、参数错误等未受理请求直接在 stderr 返回错误，不保证存在任务目录。

## 配置示例

```toml
remote_root = "~/src/macrun-demo/MyApp"
derived_data = "~/Library/Caches/macrun/MyApp/DerivedData"
project = "MyApp.xcodeproj"
scheme = "MyApp"
configuration = "Debug"
app_relative_path = "Build/Products/Debug/MyApp.app"
exclude = []
screenshot_max_edge = 1600
allow_foreground_fallback = false
sync_timeout_seconds = 120
build_timeout_seconds = 900
test_timeout_seconds = 900
launch_timeout_seconds = 60
ui_timeout_seconds = 30
```

`project` 与 `workspace` 二选一；路径中的 `~` 由 worker 展开为 Mac 用户目录。`app_relative_path` 相对于 derived_data。连接配置在两端分别维护，不放在项目配置中。后续增加 `package = true` 时，与 project/workspace 互斥；支持 swift build/test，demo 不负责把普通 Swift Package 打包成 `.app`。

## 验收与测量

先用带按钮计数器、文本框和第二窗口的简单原生应用验通，再用一个实际开发项目验证。

1. 家庭网络中的 worker 主动连接成功；Linux 不安装 Swift 或 Xcode。
2. Linux 修改未提交 Swift 文件，test/run 使用该版本；日志在任务结束前持续出现。
3. run 返回明确的产物路径、run id 和只包含被测窗口的 PNG。
4. 后台点击后计数器变化；输入后文本正确；返回新观察，由 agent 或人工确认效果。分别记录 AppKit/SwiftUI 实际支持情况，不根据上游表格直接判通过。
5. 多窗口明确选择目标，坐标点击与截图对应；旧 snapshot 返回 `stale_snapshot`。
6. 第二次只改一个小文件，只传变化文件；删除文件在镜像生效；缓存和结果不往返同步。
7. 同步中断后不进入构建；恢复网络后重新同步可修复镜像。构建失败保留日志和此前运行会话。
8. 构建中断网或 cancel，旧任务清理完成后才能开始新任务；worker/server 重启时不悄悄重复执行旧动作。
9. 忙时普通任务返回 busy，status 仍可查询。离线请求返回 worker_offline。
10. Cua 不可用时纯构建和非 UI 测试可继续；启动成功但截图失败保留 run id，之后可重试观察。
11. 未启用前台回退时明确返回后台拒绝；开启后单独记录前台运行结果，不计为后台能力通过。

记录实际 RTT、带宽、工作区规模、变化文件大小、PNG 字节数、同步/构建/捕获/回传耗时、恢复时间和内存。重复相同场景至少 10 次，先报告原始值、中位数和最大值。原有“3 秒同步、2 秒截图、3 秒动作、80MB 内存”降为观察参考，不阻塞 demo；积累数据后再决定性能门槛与是否需要压缩、块级同步。

## 产品化再处理

身份配对与轮换、代码执行隔离、Cua bounded 权限清单、严格路径防护与隐私策略、完整崩溃自动恢复、事务式整目录回滚、多项目、分发安装、签名公证和严格性能 SLA，均不作为本轮开发前置条件。demo 仍应避免误删无关文件、误操作其他应用，并明确报告无法恢复的状态。
