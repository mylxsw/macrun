# 浏览器视觉夹具

仅用于检查 **真实 React 组件** 的浮条外观、配对第二/三步和错误状态。`visual.html` 先安装内存中的 Tauri 接口替身，再加载 `src/main.tsx`；没有另一份界面实现，也没有修改产品入口。正式构建仍只使用 `desktop/index.html`，不会包含此测试页面。

在 `desktop/` 运行 `npm run dev` 后打开：

- `http://127.0.0.1:1420/tests/visual.html?overlay=1`：实际 Overlay 组件，建议视口 600×64。
- `http://127.0.0.1:1420/tests/visual.html?pairing=1`：首次配对，建议视口 800×580。粘贴 `macrun://fixture/pair?code=UI-REVIEW`，点击“连接”“继续”可检查后续两步。
- 在配对 URL 后加 `&error=pair`、`&error=start` 或 `&error=connection`，分别检查交换、启动、连接检查失败。

页面标题明确标注“UI 测试夹具（模拟连接）”。`html[data-visual-ready="true"]` 表示界面已挂载并等待本地字体加载；`window.__MACRUN_VISUAL_FIXTURE__.calls` 保留当前页面的调用记录。刷新页面会恢复初始状态。

所有响应均为明确的测试数据。脚本不连接真实 worker/服务器，不访问钥匙串、系统权限或文件，不执行命令。未支持的操作报错。配对成功截图只证明前端成功状态与跳转，不证明真实网络交换、凭据保存或系统授权。屏幕提示的浏览器截图只证明组件外观；原生窗口置顶、定位、透明度和截图保护仍须在 `.app` 中验证。
