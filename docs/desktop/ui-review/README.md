# v2 界面还原复查

2026-10-04。以 [v2 设计说明](../design.md)、七个画板的源文件与参考截图为依据；本次只还原既定布局、视觉和相关交互，没有修改设计稿。

## 七个画板

| 画板 | 设计依据 | 修改前 | 修改后 | 对照重点 |
| --- | --- | --- | --- | --- |
| 现场 | [源文件](../prototype/screens/Main.dc.html) · [参考图](../images/main.png) | [旧版](images/before/main.png) | [main.png](images/main.png) | 侧栏、三条独立状态、任务活动、右侧工作区与最近事件；统一字体、卡片与按钮密度。 |
| 任务 | [源文件](../prototype/screens/Tasks.dc.html) · [参考图](../images/tasks.png) | [旧版](images/before/tasks.png) | [tasks.png](images/tasks.png) | 左侧任务列表与右侧详情比例、搜索和状态筛选、日志与未知状态提示；保留全部九种任务状态。 |
| 桌面控制 | [源文件](../prototype/screens/Desktop.dc.html) · [参考图](../images/desktop.png) | [旧版](images/before/desktop.png) | [desktop.png](images/desktop.png) | 后端与使用条件双列、行为设置和观察区的顺序；保留真实后端、权限、工具与回放入口。 |
| 设置与安全 | [源文件](../prototype/screens/Settings.dc.html) · [参考图](../images/settings.png) | [旧版](images/before/settings.png) | [settings.png](images/settings.png) · [诊断区](images/settings-diagnostics.png) | 设置分组、连接信息、目录限制、危险命令与原生选项；长页滚动不遮挡保存和诊断操作。 |
| 首次配对 | [源文件](../prototype/screens/Pairing.dc.html) · [参考图](../images/pairing.png) | — | [pairing.png](images/pairing.png) | 独立窗口、左侧三步进度、输入与反馈；成功后进入权限和就绪步骤。 |
| 菜单栏面板 | [源文件](../prototype/screens/MenuBar.dc.html) · [参考图](../images/menubar.png) | — | [menubar.png](images/menubar.png) | 窄面板、状态与任务摘要、暂停和全部停止；点击任务打开主窗口对应详情。 |
| 屏幕提示 | [源文件](../prototype/screens/Overlay.dc.html) · [参考图](../images/overlay.png) | — | [overlay.jpg](images/overlay.jpg) | 深色浮条、橙色活动点、当前操作、停止按钮及快捷键；保留原生屏幕边框和截图保护。 |

四张修改前截图来自真实旧版应用，使用与修改后相同的隔离 IPC 数据基线。配对、菜单栏和浮条没有单独保存旧版截图，不用设计图替代旧版证据。截图左上角偶尔出现的紫色屏幕共享标记由 macOS 和截图工具显示，不属于 Macrun 界面。

主界面设计尺寸为 1280×900。本机受可用屏幕空间影响，真实主窗口截图为 1280×864；另检查了最小尺寸 760×580、长命令、长路径和多任务滚动。配对窗口为 800×580，菜单栏面板宽 352，浮条为 600×64。尺寸均为逻辑像素，原生截图可能按屏幕倍率导出。

补充截图：[任务最小尺寸](images/tasks-minimum.png)、[设置最小尺寸](images/settings-minimum.png)、[断线](images/main-offline.png)、[空状态](images/main-empty.png)、[菜单栏待确认](images/menubar-approval.png)、[原生配对错误](images/pairing-error.png)。

设置源文件使用 1400 像素长画板，而参考截图与说明标注 1440；这里保留源文件中的分组顺序和正常滚动，不把静态画板总高当作固定产品窗口高度。菜单栏与浮条原型中的演示场景切换器、假系统菜单栏和背景应用不属于产品界面。

## 截图与数据边界

现场、任务、桌面控制、设置、首次配对和菜单栏六页使用真实 Tauri 调试应用。连接、任务、工作区与后端示例通过独立临时目录中的 Unix socket 测试服务提供；产品组件和原生窗口是真实实现。测试数据不写入正式配置，服务不执行示例命令，也不访问示例服务器。说明与复现方式见 [IPC 截图夹具](../../../desktop/scripts/fixture-README.md)。

原生浮条保留 `content_protected`，因此截图接口无法可靠取得其内容。这里的浮条截图来自 [浏览器视觉夹具](../../../desktop/tests/visual-README.md)，加载与应用相同的 `main.tsx` 和 Overlay 组件；不是另写的一份 UI。该图用于核对组件外观，不能证明原生置顶、跨屏定位、透明度或截图保护的实际效果。

配对[第二步](images/pairing-step2-fixture.jpg)和[第三步](images/pairing-step3-fixture.jpg)也使用同一浏览器测试桥接，检查成功后的布局、按钮和跳转。这些图只证明前端状态流转，不证明真实网络配对、凭据保存或系统授权成功。首次配对原生截图用于核对实际窗口和输入状态；真正的配对协议由下面的 QUIC 回归测试提供有限的独立证据。

两种夹具都只存在于测试脚本或测试入口中，正式前端构建仍使用 `desktop/index.html`。没有将固定成功响应放进产品组件或正式 IPC 接口。

## 交互与构建记录

- `cd desktop && npm test`：24 项前端测试通过，覆盖模型、实际 App 与组件交互；包括九种状态筛选、命令与目录搜索、暂停和全部停止的不同调用、未知任务不重放、菜单栏任务跳转、配对与设置切换、弹窗 Tab/Escape 及焦点恢复。
- `cd desktop && npm run build`：TypeScript 检查和前端生产构建通过。本次复查包使用前端资源标识 `ZarfT9OP` / `BSonbrA2`。
- 桌面 Rust 检查通过；原生回归 2 项、QUIC 配对回归 4 项通过。
- `make desktop-build PROFILE=debug`：调试 `.app` 打包通过；截图复查使用该原生应用。产物位于 `desktop/src-tauri/target/debug/bundle/macos/Macrun Desktop.app`。

原生应用中实际点击了暂停、全部停止、菜单栏审批的“允许”和“拒绝”、浮条停止，以及菜单栏任务跳转；检查了配对错误、原生 Cmd+Q 退出确认和工具列表弹窗 Escape 关闭。断线、空状态和最小尺寸的最终界面另存为上面的补充截图。测试结束已关闭测试应用与夹具；原有正式 worker 保持运行，未迁移或重启。

界面继续区分连接、执行器和桌面控制状态；缓存快照不会把不可用的 worker 改成可用。暂停仅阻止新任务；全部停止仍暂停执行、关闭桌面调用并取消任务。`unknown` 保留“可能已生效”的提示，不提供自动重放。目录限制状态按实际开关显示，证书指纹不再标注错误的算法名称。编辑中的设置不会被后台快照覆盖。

本记录覆盖 UI 还原、相关交互和上述构建/测试。它不代表完整系统验收：真实服务器长连接、钥匙串持久化、LaunchAgent、自启动、系统权限授权和真实桌面操作的完整流程，仍以 [原生验收记录](../native-acceptance.md)及后续专门验收为准。
