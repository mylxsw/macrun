# 原生验收入口与流程

当前实现提交：`e14495a`。自动化检查通过不代表以下原生项目通过；结果以 `completion-checklist.md` 为准。

## 已查清的授权边界

当前对话通过 Telegram + MetaBot 接入。用户已经明确允许验收，但电脑操作工具仍返回 `Computer Use was not approved to use Macrun Desktop`。

2026-10-04 核对官方说明后确认：Computer Use 的应用授权与 macOS 系统权限不同，也不由 Auto-review 代替。之前将此错误称为“自动审批拒绝”不够准确；错误只能证明应用访问未获工具批准，不能证明是 MetaBot 或系统权限导致。

官方支持的原生授权入口在 ChatGPT 桌面客户端。已检查本机 `/Applications/ChatGPT.app` 存在，但尚未确认账号、版本或插件是否支持此功能，不能保证设置后本 Telegram 会话会继承授权。

- [Computer Use 设置与应用授权](https://learn.chatgpt.com/docs/computer-use)
- [Auto-review 与应用授权的区别](https://learn.chatgpt.com/docs/sandboxing/auto-review)

## 用户需要做的最少操作

1. 打开 Mac 上的 ChatGPT 客户端，进入 Codex 或 Work。
2. 在 Plugins 中打开 Computer Use；若尚未启用，按照客户端提示安装或启用。系统权限只在客户端明确要求时处理。
3. 打开本项目，发送下面的验收接续指令。出现针对 Macrun Desktop 的应用访问请求时，由用户批准该应用。无需开放所有应用。
4. 若没有 Computer Use 功能或无法出现授权，记录实际显示的提示；不要通过关闭沙箱、修改默认权限或其他自动化工具绕过拒绝。

可复制的接续指令：

```text
继续验收 /Users/mylxsw/Workspace/codes/vibe/macrun。
先阅读 docs/desktop/completion-checklist.md、native-acceptance.md 和 design.md。
代码已推送，当前实现提交 e14495a。不要把构建成功当作原生验收通过。
请使用 Computer Use 检查 Macrun Desktop，并在工具要求时向我发起该应用的访问授权。
应用路径：/Users/mylxsw/Workspace/codes/vibe/macrun/desktop/src-tauri/target/debug/bundle/macos/Macrun Desktop.app
先确认测试隔离，不接管现有 LaunchAgent，不连接生产服务，不展示或导出凭据。
逐项完成原生验收、修复问题、回归验证，并提交推送。若权限仍拒绝，记录原始错误，不绕过。
```

## 验收顺序与证据

每一项记录实际结果、系统版本、失败重现步骤；截图必须避免包含邀请、凭据及私人桌面内容。

| 顺序 | 实际操作 | 通过条件 |
| --- | --- | --- |
| 1 | 首次启动、四页导航、缩小窗口、滚动与弹窗 | 与 v2 设计相符，无遮挡、空白或不可操作控件 |
| 2 | 隔离服务生成邀请，完成配对与连接检查，关闭并重开 | 钥匙串凭据可取回；界面显示真实连接结果；失效邀请错误清楚 |
| 3 | 执行测试任务，查看输出和工作区，批准、拒绝、等待超时 | 状态和实际执行一致；拒绝或超时不执行 |
| 4 | 菜单栏打开、关闭主窗口、暂停、全部停止、停止快捷键 | 关闭窗口保留 worker；停止取消任务；退出后无遗留 worker |
| 5 | 测试后端实拍与观察回放，检查屏幕提示与边框 | 真实截图可查看；截图排除效果以实际后端结果为准 |
| 6 | 硬件输入让出、防休眠、通知、诊断导出 | 仅支持且获准的能力启用；诊断不包含敏感信息 |
| 7 | 隔离旧服务迁移、失败回滚、登录启动、异常退出恢复 | 测试服务正确切换；失败可恢复；不重放未知任务 |

最后运行受影响的自动化检查并重新构建。所有原生项通过前，不发布正式版、不将测试包当成完整交付。
