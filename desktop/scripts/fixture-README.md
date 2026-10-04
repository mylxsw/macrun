# UI 截图夹具

`fixture-worker.py` 是独立的本机测试服务，只通过现有的 Unix socket 协议向真实 Tauri 应用提供示例数据。它不运行命令、不访问示例服务器，也不修改 LaunchAgent、钥匙串或正式配置。发布应用不包含或依赖该脚本。

先初始化一个临时目录，再启动服务：

```sh
uv run desktop/scripts/fixture-worker.py init --data /tmp/macrun-ui-v2
uv run desktop/scripts/fixture-worker.py serve --data /tmp/macrun-ui-v2
```

在另一终端启动 debug 应用。`MACRUN_DESKTOP_DATA` 必须与初始化时完全一致：

```sh
MACRUN_DESKTOP_DATA=/tmp/macrun-ui-v2 \
  desktop/src-tauri/target/debug/bundle/macos/'Macrun Desktop.app'/Contents/MacOS/macrun-desktop
```

连接、任务、工作区和后端快照来自夹具；它不能证明真实服务器握手或执行引擎运行成功。原生系统权限、钥匙串和 LaunchAgent 检查仍使用本机真实状态。不要为截图绕过现有的旧执行器保护。

默认自动连接、通知、防休眠和输入让出均关闭，避免测试启动干扰日常工作；橙色提示只在 `overlay` 场景出现。

切换场景会重置该场景的修改，已有订阅立即更新，无需重启应用：

```sh
uv run desktop/scripts/fixture-worker.py state --data /tmp/macrun-ui-v2 --state approval
```

可用场景：`working`、`idle`、`empty`、`paused`、`offline`、`approval`、`overlay`、`long`。运行时长固定在相同相对值，便于修改前后截图比较；显示时间基于当前本地时间。`long` 含长命令、长路径及 40 条任务。

支持真实界面发出的暂停、桌面开关、全部停止、单条取消、批准／拒绝、安全设置、任务详情、工具列表、重启后端和退出请求。所有动作记录到临时目录的 `fixture-actions.jsonl`。未实现的动作明确报错，不返回虚假的成功。截图、配对握手、系统权限、钥匙串和 LaunchAgent 都不由此脚本模拟。

可以直接检查状态或调试一个已有接口：

```sh
uv run desktop/scripts/fixture-worker.py control --data /tmp/macrun-ui-v2 --action snapshot
uv run desktop/scripts/fixture-worker.py control --data /tmp/macrun-ui-v2 --action pause --args '{"paused":true}'
```

用 `Ctrl+C` 停止外部服务。初始化拒绝正式路径及不带夹具标记的已有目录；服务拒绝占用已有 socket。首次配对界面用新的目录初始化，加 `--unpaired`，然后启动应用即可。再次初始化已存在的目录不会删除已有连接信息，重新检查首次配对请另建临时目录。
