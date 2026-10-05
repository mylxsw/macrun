# Macrun 0.2 设计

## 职责

- `main/frontend`：JSON CLI 与 stdio MCP；MCP 工具定义、图片内容输出；CLI 分块传文件。
- `server`：本机 Unix socket 接口、多个 QUIC peer 注册、转发通用请求、源目录扫描与同步任务结果。
- `worker`：主动连接、心跳重连、接收同步流，向持续存活的 Engine 分派通用请求。
- `engine`：worker 所有的任务、去重、结果落盘、日志查询、取消、backend 会话。
- `backend`：通用 stdio MCP 初始化、序列化 JSON-RPC、持久进程和 session generation。
- `files`：远程目录、分块读写、图片、重命名。
- `sync/wire/process`：增量目录协议、QUIC 帧、原子 JSON、进程组管理。

## 两类任务

命令和 MCP 调用的 task_id 由调用者的 UUID request_id 指定。worker 先记录 accepted，再启动异步任务；请求参数同编号去重。task.get 返回状态和持久输出。服务器失联不会取消任务，结果查询可在服务器重启后重新路由到 worker。

同步由服务器负责源目录，使用服务器生成的 job_id，结果在 server-data/sync-jobs。同步与普通命令可在不重叠的工作区并存，重叠目录通过 worker 进程级租约互斥；Agent 可把命令绑定到同步返回的 generation。如果服务器重启，旧同步标记失败，必须重新同步。新端允许同 worker 的独立工作区同时同步；旧协议仍串行。

## 断线与重启

网络断开：任务、backend 和取消令牌属于 Engine，不属于 QUIC session。心跳重连只重建传输。不自动重放请求。确认任务使用相同 UUID；发送方未拿到回复时结果可能未知。

worker 重启：持久 accepted/running 记录标记 unknown；对命令的 process.json 尝试按进程身份清理旧进程组。无法确认的清理错误保留在 recovery 中，不声称已清理。已完成记录保留。任务管理不是独立系统服务，worker 崩溃不承诺任务继续运行。

同一 request_id + 不同参数拒绝；task_id 与 worker 状态目录绑定。删除状态目录会失去去重历史。任务副作用发生后、保存结果前崩溃，无法保证 exactly-once，只能返回 unknown；不对点击/输入盲目补发。

## MCP

每个配置 backend 对应一个持久 stdio 进程，用 Mutex 串行请求。mcp.tools 返回工具定义及随机 session。mcp.call 必须携带该 session；网络重连保持 session，backend 重建更换 session，旧调用失败并要求重新观察。

长调用在后台任务中等待最终 MCP 结果，无固定 120 秒 relay 超时。调用有可配置总超时（默认 300 秒），锁等待也有超时。取消或协议错误会丢弃该 backend 连接；GUI 常驻 daemon 可能仍存活，因此动作效果可能未知。

不实现 resources/prompts/反向客户端回调或进度通知转发；backend 发起客户端请求会收到不支持错误。tools/list 保留 nextCursor，调用者传 cursor 分页。tool isError 返回值原样保留，任务标记 failed。

前端 MCP task_get 拆出 backend content，保持文本/图片的内容块类型，避免把 PNG 仅作为 JSON 字符串交给模型。大于帧限制的结果需要通过文件工具另取。

## 目录和文件

同步 config 与命令 cwd 独立。watch 由文件系统事件触发并定期严格校验；每轮检查元数据、传变化文件并确认源端清单，内容哈希可缓存；退出 watch 不撤销已发起的同步。轮询重试可修复中断状态；不是原子工作目录快照。

文件读取按字节偏移，二进制用 base64。图片限 8 MiB，QUIC JSON 帧限 16 MiB，文件块限 512 KiB。下载使用大小/mtime 变化检测（不是全文件快照锁），上传写临时文件后重命名，失败留临时文件；新端 CLI 上传支持同 transfer_id 的前缀校验续传，旧端保留分块路径。file.write 可直接覆盖，调用者负责避免与同步相互覆盖。

## 兼容性

v0.2 协议 2，与 v0.1 不互通。保留证书/token 身份方式，不做安全产品化扩展。QUIC 是否适用于真实美中网络要独立验证；当前没有 TCP fallback。电脑的 GUI 会话与本地 MCP 安装仍是使用前提。


## 性能协议扩展

协议 2 的可选能力协商、连续文件流、清单摘要与分页、哈希缓存、工作区租约/generation、图片产物分离和任务等待，见 [performance.md](performance.md)。该文档描述新能力与旧端兼容路径的准确边界。
