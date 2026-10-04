# Macrun Desktop（M0）

按照 `docs/desktop/` 的 v2 设计开发。Tauri 2 + React + TypeScript，托管现有 Rust worker。当前支持 macOS；Windows 尚需替换 Unix IPC、进程管理和系统集成。

## 开发

需要 Node.js 22、npm、Rust 和 Xcode Command Line Tools。Rust 可使用项目 `scripts/deps.sh` 安装。

```sh
cd desktop
npm ci --include=dev
npm run desktop:dev
```

在“设置与安全”填写已有服务器地址、证书文件、令牌文件和可选 worker TOML 配置；保存后启动连接。配置只保存文件路径。一次性邀请配对和钥匙串尚未接入，不要把真实配置或凭据提交到仓库。

应用使用独立的应用数据目录。检测到旧 `dev.macrun.worker` LaunchAgent 时拒绝启动新 worker，不会停止或接管旧服务。开发版可用 `MACRUN_DESKTOP_DATA` 指定隔离数据目录。

```sh
npm run desktop:build -- --debug
# 产物：src-tauri/target/debug/bundle/macos/Macrun Desktop.app
# 正式优化构建（仍需另外完成签名、公证和发布验收）：
npm run desktop:build
```

构建脚本会将对应 profile 的 worker 一起打包。关闭窗口隐藏到菜单栏；退出应用会请求停止 worker。应用崩溃后 worker 通过父进程管道关闭触发清理。此机制不承诺睡眠、注销或系统重启期间继续执行任务。

## 验证

```sh
npm test
npm run build
cd ..
make check
./scripts/cargo-local.sh build --locked
python3 scripts/desktop-smoke.py
./scripts/cargo-local.sh clippy --manifest-path desktop/src-tauri/Cargo.toml --locked --all-targets -- -D warnings
```

联调只使用隔离临时目录、本机临时端口及测试 MCP 后端，不连接生产服务。

## 当前边界

实现现场、任务、桌面控制、设置四页和菜单栏面板；数据来自 worker。浏览器单独运行仅显示未连接状态，不注入模拟任务。任务展示最近 200 条，输出展示末尾 8 KiB，可打开完整日志。

暂停只阻止新任务，已运行任务继续；全部停止同时暂停接收、关闭桌面控制并取消任务。无法确认结果的桌面操作标记为 unknown，不自动重放。桌面开关不限制普通 shell 命令。

私有 IPC 校验同用户且仅开放固定控制动作，但不是针对同用户 shell 的安全隔离。命令仍使用本机用户权限。环境变量值从任务参数中遮盖，命令输出或命令字符串里的秘密不会自动消除。

未实现：邀请配对、钥匙串、实际系统权限自检、旧服务迁移、审批、目录限制、屏幕提示、操作回放、截图核对、自动恢复 worker。登录启动只启动应用，连接仍需手动启动。

原生窗口、托盘、快捷键、登录启动和实际权限流程需要在允许电脑操作的环境中手动验收；编译成功不代表这些交互已验证。详见 [开发记录](../docs/desktop/development.md)。
