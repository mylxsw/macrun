# UI 截图夹具

`fixture-worker.py` 是独立的本机测试服务，只通过现有的 Unix socket 协议向真实 Tauri 应用提供示例数据。它不运行命令、不访问示例服务器，也不修改 LaunchAgent、钥匙串或正式配置。发布应用不包含或依赖该脚本。

先初始化一个临时目录，再启动服务：

```sh
uv run desktop/scripts/fixture-worker.py init --data /tmp/macrun-ui-v2
uv run desktop/scripts/fixture-worker.py serve --data /tmp/macrun-ui-v2
```

使用独立的调试应用副本，通过 Finder 或 Computer Use 的应用入口启动。先退出当前 Macrun 应用，再准备副本；`MACRUN_DESKTOP_DATA` 必须与初始化时完全一致：

```sh
mkdir -p .local
fixture_bundle_dir=$(mktemp -d "$PWD/.local/macrun-ui.XXXXXX")
fixture_app="$fixture_bundle_dir/Macrun UI Check.app"
ditto 'desktop/src-tauri/target/debug/bundle/macos/Macrun Desktop.app' "$fixture_app"
python3 - "$fixture_app" /tmp/macrun-ui-v2 <<'PY'
import pathlib, plistlib, sys
info = pathlib.Path(sys.argv[1]) / 'Contents/Info.plist'
with info.open('rb') as source:
    plist = plistlib.load(source)
plist.setdefault('LSEnvironment', {})['MACRUN_DESKTOP_DATA'] = sys.argv[2]
with info.open('wb') as destination:
    plistlib.dump(plist, destination)
PY
codesign --force --deep --sign - "$fixture_app"
codesign --verify --deep --strict "$fixture_app"
printf '%s\n' "$fixture_app"
```

在 Finder 中打开最后输出的完整应用路径，或把该路径传给 Computer Use 的 `getApp`。不要修改 `/Applications` 中的正式应用。这里的 ad-hoc 签名仅用于本机测试，不代表正式分发签名或公证。

2026-10-04 实测：直接从 shell 执行 bundle 内的二进制时，辅助功能树可访问，但窗口实际为白屏；同一 debug 二进制按上述方式设置独立数据目录并通过系统应用入口启动后正常绘制。因此直接执行二进制不作为可靠的 UI 验收启动方式。这是启动方式的对比结果，尚未定位底层渲染原因。

连接、任务、工作区和后端快照来自夹具；它不能证明真实服务器握手或执行引擎运行成功。原生系统权限、钥匙串和 LaunchAgent 检查仍使用本机真实状态。不要为截图绕过现有的旧执行器保护。

默认自动连接、通知、防休眠和输入让出均关闭，避免测试启动干扰日常工作；橙色提示只在 `overlay` 场景出现。

切换场景会重置该场景的修改，已有订阅立即更新，无需重启应用：

```sh
uv run desktop/scripts/fixture-worker.py state --data /tmp/macrun-ui-v2 --state approval
```

可用场景：`working`、`idle`、`empty`、`paused`、`offline`、`approval`、`overlay`、`long`、`large`。场景初始化时固定任务时间，保证跨页游标稳定；重新切换场景会刷新这些时间。`long` 含长命令、长路径及 40 条任务；`large` 含 1500 条任务、九种任务状态、24 个工作区，以及任务错误对象和文本输出。

快照与生产接口一致，只保留最近 200 条及全部活动任务，并返回全量 `task_counts`。`task_list` 支持状态、搜索、kind 和稳定游标分页，默认 50 条、最多 100 条；摘要不携带输出或图片，`task_detail` 按需读取输出，失败记录中的 `error` 保留为对象，真正操作错误使用字符串。

支持真实界面发出的暂停、桌面开关、全部停止、单条取消、批准／拒绝、安全设置、任务详情、工具列表、重启后端和退出请求。所有动作记录到临时目录的 `fixture-actions.jsonl`。未实现的动作明确报错，不返回虚假的成功。截图、配对握手、系统权限、钥匙串和 LaunchAgent 都不由此脚本模拟。

大数据验收可使用独立目录：

```sh
uv run desktop/scripts/fixture-worker.py init --data /tmp/macrun-ui-acceptance --state large
uv run desktop/scripts/fixture-worker.py serve --data /tmp/macrun-ui-acceptance
```

准备应用副本时，也将上述 `python3` 命令的临时目录参数改为 `/tmp/macrun-ui-acceptance`。本轮使用的副本是 `.local/ui-acceptance/Macrun UI Check.app`。

搜索 `fixture-task-1486` 能找到快照之外的历史并读取失败详情；各状态计数应来自完整的 1500 条记录。

可以直接检查状态或调试一个已有接口：

```sh
uv run desktop/scripts/fixture-worker.py control --data /tmp/macrun-ui-v2 --action snapshot
uv run desktop/scripts/fixture-worker.py control --data /tmp/macrun-ui-v2 --action pause --args '{"paused":true}'
```

用 `Ctrl+C` 停止外部服务。初始化拒绝正式路径及不带夹具标记的已有目录；服务拒绝占用已有 socket。首次配对界面用新的目录初始化，加 `--unpaired`，然后启动应用即可。再次初始化已存在的目录不会删除已有连接信息，重新检查首次配对请另建临时目录。
