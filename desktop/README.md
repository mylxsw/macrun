# Macrun Desktop

Tauri 2 + React + TypeScript，按 `docs/desktop/` v2 设计开发，托管现有 Rust worker。当前实现面向 macOS，Windows 仍是独立的平台适配任务。

当前状态：v2 七个画板已完成一轮原生／浏览器还原检查，旧版配置迁移已实机执行。当前继续检查大量任务、操作反馈、桌面控制和权限；具体证据及未完成项目见 [界面与交互验收](../docs/desktop/quality-review.md)。不要把构建成功视为正式发布完成。

## 构建与开发

在项目根目录直接运行：

```sh
make desktop-build                # 编译优化版 .app
make desktop-dev                  # 启动开发应用，前端热更新
make desktop-build PROFILE=debug  # 编译更快的调试版 .app
```

首次使用需要安装 Node.js 22.12+（包含 npm）和 Xcode Command Line Tools（`xcode-select --install`）。命令会检查环境，缺少 Rust 时沿用项目脚本安装；自动安装锁定版本的前端依赖，并构建、打包 worker。依赖未变化时跳过 npm 安装。首次下载需要联网。

优化版应用位于 `desktop/src-tauri/target/release/bundle/macos/Macrun Desktop.app`，调试版将 `release` 换成 `debug`。在访达中双击即可运行。`desktop-dev` 在前台运行，按 Ctrl+C 停止；它启动本机桌面端，不自动创建远端服务，首次连接仍需要配对。开发模式会记住本机配置，已开启自动连接时会按已有配置连接。

下面的 npm 命令仍可直接使用：

```sh
cd desktop
npm ci --include=dev
npm run desktop:dev
# 开发安装包
npm run desktop:build -- --debug
# 优化安装包；签名、公证仍需单独配置
npm run desktop:build
```

应用与对应 profile 的 worker 一起打包。开发包位于 `src-tauri/target/debug/bundle/macos/Macrun Desktop.app`。开发隔离可设置 `MACRUN_DESKTOP_DATA`，此变量仅在 debug 构建使用。

## 连接

在服务器生成十分钟一次性邀请：

```sh
macrun invite --data /path/to/server-state --server server.example:7443
```

将 `macrun://pair/…` 粘贴到桌面端。邀请含公开证书、BLAKE3 指纹与一次性码，不含长期令牌。通过固定证书建立 QUIC 后兑换独立设备凭据；服务器只存设备凭据哈希，桌面凭据进入 macOS 钥匙串，经继承管道注入 worker，不放在进程参数或环境变量里。邀请已兑换或交换途中中断时，请生成新邀请。

也可手动填写服务器、证书和令牌文件。保存时复制并固定证书，将凭据导入钥匙串；原文件不会被删除。后端配置在桌面控制页编辑，保存前断开连接，重新连接生效。

一期优先跑通：新桌面数据目录默认开启桌面控制、命令直接执行，桌面三档工具都“允许并提示”。已有数据目录保留原来保存的设置，不会被自动放宽；需要时在设置与安全、桌面控制页调整。原 CLI 默认行为不变。工作目录限制需要在设置中启用并配置允许目录。

## 行为

- 暂停只阻止新任务；全部停止同时暂停、关闭桌面控制、取消任务及后端调用。
- 命令确认支持直接、风险、每条三档。等待确认 60 秒后拒绝，拒绝原因写入 `error.code`：用户拒绝为 `approval_rejected`，无人处理为 `approval_expired`，重启中断为 `approval_interrupted`。重启后未处理的确认直接拒绝，不执行。
- 确认时可选“允许一次”“15 分钟内允许同类”（同一程序且在该目录树内；桌面为同一后端的同一工具）或“本次运行允许”（该目录树内所有命令；桌面为同一后端的同一档工具）。临时允许只保存在执行器内存，重启即失效，可在设置与安全中查看和撤销。
- 桌面工具按后端声明分三档：只读为“观察”，后端风险等级 `r3` 的非只读工具为“高风险”，其余为“操作”。每档可设为允许并提示、先确认或禁止；禁止时调用直接返回 `desktop_denied`。桌面应用自己的观察请求不需要确认。
- 风险模式只放行少量简单只读命令，未知程序、解释器、脚本和 shell 操作符要求确认。它不是安全沙箱。
- 目录检查覆盖 cwd、文件路径、同步目标、已有符号链接与 `..`。Shell 仍以本机用户权限运行，同用户进程可以访问本机资源。若需要强隔离，应使用独立执行用户或系统沙箱。
- 任务页每页显示 50 条，搜索和状态筛选覆盖保留期内全部记录，按需读取详情及最后 8 KiB 输出。现场快照保留最近 200 条摘要和全部活动任务；今日计数覆盖全部今日记录。历史截图与完整输出不随每次快照重复读取，可在详情／回放按需打开。
- 保留策略为 7/30/90 天，启动及每小时清理过期任务，设置页也可手动清理。去重编号与指纹单独保留，过期任务不会因清理而再次执行。
- 工作区索引独立保存。桌面结果保留后端图片与参数，支持按任务查看；unknown 不重放，可转至后端实拍核对。
- 桌面操作时有屏幕提示、全部停止和防自动休眠；获准监听本机输入后让出 30 秒。程序合成事件不按本机输入处理。权限不可用时界面如实显示不可用。
- 屏幕提示窗口设置为受保护内容，并在已识别截图／观察调用期间隐藏；是否被具体后端捕获仍需该后端实拍验收，不能仅凭窗口配置保证。
- 配置可控制屏幕提示、输入让出、通知、防休眠及启动自动连接。待确认、失败、超时、未知结果可发不包含命令内容的通知。
- 关闭主窗口隐藏到菜单栏。退出请求停止 worker，父进程异常退出由管道关闭触发清理。崩溃恢复最多连续三次，间隔至少五秒；稳定运行一分钟后恢复重试预算。不会重放未知任务。
- 迁移前检查旧任务、备份 plist，停止并停用旧 LaunchAgent，导入连接凭据，并复制任务去重记录、同步状态和后端配置，保留命令搜索路径。旧任务目录保留；失败时尝试恢复旧配置和服务。迁移后从确认弹窗启动连接。修复原因与验证见 [迁移记录](../docs/desktop/migration-repair.md)。
- 诊断导出为允许字段组成的 JSON，不包含命令、输出、截图、路径、服务器地址或凭据。

## 验证

```sh
# 仓库根目录
make check
make smoke PROFILE=debug
python3 scripts/desktop-smoke.py
make cross-smoke PROFILE=debug
# 桌面目录
npm test
npm run build
# 仓库根目录
./scripts/cargo-local.sh clippy --manifest-path desktop/src-tauri/Cargo.toml --locked --all-targets -- -D warnings
./scripts/cargo-local.sh test --manifest-path desktop/src-tauri/Cargo.toml --locked
```

自动化使用隔离测试数据、本机端口或临时 Linux Docker 服务；MCP fixture 返回图片用于协议测试，不代表真实桌面截图已验收。具体状态见 [完整验收清单](../docs/desktop/completion-checklist.md)。
