# 浏览器视觉夹具

仅用于检查 **真实 React 组件** 的首次入口、迁移对话框、浮条外观、配对第二/三步和错误状态。`visual.html` 先加载 `visual.ts`，安装内存中的 Tauri 接口替身，再加载 `src/main.tsx`；没有另一份界面实现，也没有修改产品入口。正式构建仍只使用 `desktop/index.html`，不会包含此测试页面。

在 `desktop/` 运行 `npm run dev` 后打开：

- `http://127.0.0.1:1420/tests/visual.html`：已有桌面配置且已连接，应进入“现场”。建议视口 1280×900。
- `http://127.0.0.1:1420/tests/visual.html?legacy=running`：桌面配置为空、检测到正在运行的旧版，应进入设置页，顶部显示迁移入口。
- `http://127.0.0.1:1420/tests/visual.html?legacy=stopped`：桌面配置为空、检测到已停止的旧版，同样应进入设置页并显示迁移入口。
- `http://127.0.0.1:1420/tests/visual.html?overlay=1`：实际 Overlay 组件，建议视口 600×64。
- `http://127.0.0.1:1420/tests/visual.html?pairing=1`：首次配对，建议视口 800×580。粘贴 `macrun://fixture/pair?code=UI-REVIEW`，点击“连接”“继续”可检查后续两步。
- 在配对 URL 后加 `&error=pair`、`&error=start` 或 `&error=connection`，分别检查交换、启动、连接检查失败。
- 在旧版 URL 后加 `&error=migration` 检查迁移失败；加 `&error=start` 检查迁移成功后启动失败。每次只使用一种入口参数。

迁移回归步骤：打开旧版入口，点击“迁移”，先取消并确认仍显示旧版提示；重新打开并确认迁移，应显示“迁移完成”和测试备份路径；点击启动桌面连接，应显示启动成功。失败入口确认后应保留错误说明、重试入口和原来的旧版状态。`legacy=running` 与 `legacy=stopped` 均需检查，首次配对与已有配置入口不能受到迁移分流影响。

模拟的 `migrate_legacy` 只修改当前页面内存中的 `settings`、`legacy_detected` 和 `legacy_running`，返回 `/tmp/macrun-visual-fixture/` 下的假备份路径，不创建任何文件。`error=migration` 会抛出测试错误并保持状态原样。迁移不会自动启动；对话框随后复用模拟的 `start_worker`，更新内存中的运行状态和快照，并发送 `worker-state` 事件。`error=start` 会在更新这些状态之前失败。

页面标题明确标注“UI 测试夹具（模拟连接）”。`html[data-visual-ready="true"]` 表示界面已挂载并等待本地字体加载；`window.__MACRUN_VISUAL_FIXTURE__.calls` 保留当前页面的调用记录。刷新页面会恢复初始状态。

所有响应均为明确的测试数据。脚本不连接真实 worker/服务器，不访问钥匙串、系统权限或文件，不执行命令。未支持的操作报错。配对成功截图只证明前端成功状态与跳转，不证明真实网络交换、凭据保存或系统授权。屏幕提示的浏览器截图只证明组件外观；原生窗口置顶、定位、透明度和截图保护仍须在 `.app` 中验证。
