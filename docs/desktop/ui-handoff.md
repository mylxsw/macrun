# Macrun UI 还原交接

## 给接手的 ChatGPT / Codex

用户最新反馈：**整个 UI 还原度比较差，请按已重新设计的 v2 设计稿修正。** 本次任务是还原既定设计，不是重新设计产品。先看全部设计画板，再打开真实应用，逐页比较、修改并复查。不要只读文档、只改配色或只完成首页就宣告完成。

用户暂时不要求完成此前全部系统功能验收；本次重点是 UI 还原和相关交互回归。无需重新讨论 Tauri / Flutter 选型。界面工作应保留已接通的功能与真实数据。

## 项目与基线

- 项目：`/Users/mylxsw/Workspace/codes/vibe/macrun`
- 远端：`https://github.com/mylxsw/macrun`，分支 `main`。
- 本文编写时最新实现提交：`b64ea06`（Make 编译／开发入口）；完整功能实现提交 `e14495a`。
- 编写交接前工作区干净。接手时重新检查 Git 状态，保留用户后续修改。
- 技术栈：Tauri 2、React 19、TypeScript、Vite；Rust worker 独立进程，经本机 IPC 接入桌面端。
- 工作区规则：先读 `/Users/mylxsw/Workspace/codes/AGENTS.md`，再检查仓库内新增规则。

## 设计依据：全部读取与查看

相对于仓库根目录：

| 画板 | 设计源文件 | 参考截图 |
| --- | --- | --- |
| 现场 | `docs/desktop/prototype/screens/Main.dc.html` | `docs/desktop/images/main.png` |
| 任务 | `docs/desktop/prototype/screens/Tasks.dc.html` | `docs/desktop/images/tasks.png` |
| 桌面控制 | `docs/desktop/prototype/screens/Desktop.dc.html` | `docs/desktop/images/desktop.png` |
| 设置与安全 | `docs/desktop/prototype/screens/Settings.dc.html` | `docs/desktop/images/settings.png` |
| 菜单栏面板 | `docs/desktop/prototype/screens/MenuBar.dc.html` | `docs/desktop/images/menubar.png` |
| 首次配对 | `docs/desktop/prototype/screens/Pairing.dc.html` | `docs/desktop/images/pairing.png` |
| 屏幕提示 | `docs/desktop/prototype/screens/Overlay.dc.html` | `docs/desktop/images/overlay.png` |

还需阅读：

- `docs/desktop/design.md`：信息架构、交互、视觉方向。
- `docs/desktop/prototype/index.html`：可交互的完整原型，离线可打开。
- `docs/desktop/prototype/shell.html`、`build.cjs`、`check.cjs`：原型包装与生成方式。
- `docs/desktop/README.md`：画板入口和操作示例。

**不要修改设计稿来迁就当前实现。** 以 v2 源文件、原型和截图为还原依据；若它们存在冲突，先记录具体差异再处理，不默认当前代码正确。原型里的顶部画板切换器、示例数据和用于展示状态的切换按钮不属于产品界面。

## 快速启动

```sh
cd /Users/mylxsw/Workspace/codes/vibe/macrun
make desktop-dev
```

包含依赖准备、worker 编译、Tauri 开发应用和前端热更新；Ctrl+C 停止。只运行 `npm run dev` 是前端服务器，不等于启动完整 Tauri 应用。

```sh
make desktop-build PROFILE=debug  # 较快的调试构建，已实际通过
make desktop-build                # 优化构建
```

调试应用路径：

```text
/Users/mylxsw/Workspace/codes/vibe/macrun/desktop/src-tauri/target/debug/bundle/macos/Macrun Desktop.app
```

当前开发模式沿用本机配置，可能按已有设置自动连接。需要隔离时，可在 debug 模式使用项目已经支持的 `MACRUN_DESKTOP_DATA` 指定测试目录；先检查旧服务冲突，不迁移生产 LaunchAgent、不修改生产凭据。不把模拟数据写进正式配置或业务代码。

## 实现文件导航

| 文件 | 修改关注点 |
| --- | --- |
| `desktop/src/main.tsx` | App、四页布局、导航、任务列表／详情、菜单栏和悬浮条分支、SettingsPage |
| `desktop/src/Features.tsx` | Approvals、Pairing、SafetyPanel、WorkspaceList、BackendPanel、ToolResult、Replay |
| `desktop/src/style.css` | 全局样式、字体、布局、控件及状态视觉 |
| `desktop/src/types.ts`、`model.mjs` | 数据契约、状态、筛选与统计；调整 UI 时保留语义 |
| `desktop/src-tauri/tauri.conf.json` | 主窗口尺寸、标题栏、CSP |
| `desktop/src-tauri/src/main.rs` | 窗口创建、菜单栏定位、屏幕提示、事件及前端命令接线 |
| `desktop/src-tauri/src/native.rs` | 系统权限、工作区打开、配对相关检查、诊断等原生功能 |
| `desktop/tests/` | 模型与组件回归 |

当前主窗口配置为 1280×900，最小 760×580；菜单栏面板为 360×560；悬浮条为 600×64。它们是实现现状，不是要求设计稿适配这些尺寸。

## 优先检查的差异线索

下面是源码核对得到的线索，**不是已经完成的实机视觉审查**。上一执行环境没有取得 Macrun 的应用访问授权，因此尚无可信的实现截图对比。

1. **字体**：设计稿加载 Geist / Geist Mono；实现 CSS 只声明字体族，当前 `style.css` 和 `index.html` 未见字体加载。检查实际渲染字体，优先考虑本地打包字体，确保离线一致。不要未经核对扩大 CSP 来依赖外部字体。
2. **配对入口**：设计要求独立首次配对体验；当前 `Pairing` 放在 `SettingsPage` 顶部。核对首次启动、已配对后和重新配对时的结构，不能仅在设置页堆叠表单代替画板。
3. **现场布局**：当前 `WorkspaceList` 在现场内容区的右侧 `aside`，和“刚刚”共用一列。应逐项对照 Main 画板的分区、比例、顺序和留白。
4. **附加功能的布局**：BackendPanel、Replay、审批与安全设置已加入实现。保留其能力，但检查是否因直接堆叠破坏原稿的信息层级和页面密度。
5. **原生窗口**：检查标题栏与交通灯位置、拖动区域、菜单栏弹出位置、窗口边缘、圆角和阴影。不要把设计稿里的假交通灯与原生交通灯叠加。
6. **统一视觉参数**：设计基准包括主背景 `#FBFBFA`、侧栏 `#F2F2EF`、正文 `#1A1A19`、分隔线 `#E6E5E1`；正文约 13.5px，按钮约 32px 高，卡片约 10px 圆角。精确尺寸以各画板 CSS 为准，不仅匹配这几个颜色。

建议先建立逐页差异清单：结构与比例 → 字体与密度 → 间距与对齐 → 控件细节 → 状态和交互。每项标注修改文件及修改后截图，不以“感觉更好看”作为通过条件。

## 必须保留的行为

- 连接、执行器、桌面控制三条状态独立；统计不是首页主角。
- 实际已有九种任务状态：原设计七种，加 `awaiting_approval` 与 `denied`。这是功能发展，不要为了照搬旧文案删掉新状态。
- 暂停只阻止新任务；全部停止同时暂停、关闭桌面调用并取消任务。
- `unknown` 明确提示可能已生效，不自动重放；保留核对入口。
- 配对、钥匙串、审批、目录限制、输出、同步进度和后端操作继续接真实接口；不要用固定成功提示替换。
- 关闭窗口保留菜单栏；退出停止 worker；有任务时退出确认。
- 键盘焦点、弹窗 Escape/Tab、按钮禁用和错误状态可用；快照刷新不覆盖正在编辑的设置。

可以重构前端组件来提高还原度。除窗口尺寸、定位或必要交互接线外，避免顺带改通信协议和执行引擎。

## 验证与交付

1. 先保留修改前应用截图；在相同窗口尺寸和相同数据状态下与设计稿对比。离线空白页面不能拿来和设计稿的工作中状态直接比较。
2. 检查全部七个画板；覆盖断线、空闲、工作中、暂停、待确认，以及任务失败／未知、配对错误与成功等相关状态。可用隔离测试服务或明确标注的测试夹具重现，不能让正式应用永久依赖假数据。
3. 检查主窗口默认与最小尺寸、长命令／路径、多任务滚动；界面不截断关键操作，不出现双重滚动或重叠。
4. 运行 `cd desktop && npm test && npm run build`。原生代码有变动时运行桌面 Rust 检查；最后 `make desktop-build PROFILE=debug`，再在实际 .app 中复查。
5. 按页提供修改后截图和对比结论，列出未覆盖的状态。构建或组件测试通过不能代替视觉核对。
6. 更新项目文档，提交并推送本次修改，记录提交号。不要将 UI 还原完成写成全部系统验收完成。

## 前次权限问题

Telegram + MetaBot 环境曾返回 `Computer Use was not approved to use Macrun Desktop`，用户已口头确认但工具仍拒绝。不要假设当前官方客户端仍然相同；在当前客户端按正常 Computer Use 流程发起该应用的访问请求，用户批准后再检查。若仍拒绝，记录原始错误，不通过其他通道绕过。

`native-acceptance.md` 记录此前完整系统验收流程，可参考，但本次用户优先要求 UI 还原，不必把所有系统验收作为修改界面的前置条件。`development.md` 含历史 M0 记录，已过时的未实现列表不能当作当前源码现状。
