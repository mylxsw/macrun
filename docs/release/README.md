# 自动发布与首次配置

入口：GitHub → Actions → **Release** → Run workflow，分支选 **main**。
填写已存在的稳定标签 `vX.Y.Z`。首次保持 `publish` 不勾选，生成草稿供安装验收；
以后勾选即可在所有平台通过检查后自动正式发布。不支持预发布标签。

Actions 仅在主动启动此发布流程时运行。向 main 或其他分支提交／合并、创建或更新 PR、
推送标签，都不会自动触发工作流。日常开发请在本地运行 `make check`、桌面测试等检查；
发布时统一执行测试、覆盖率门槛和各平台构建。发布脚本检查只运行一次，通过后才启动
各架构构建，避免重复消耗额度。

## 需要维护者准备的信息

先打开仓库的 [Actions Secrets](https://github.com/mylxsw/macrun/settings/secrets/actions)，
逐项点 **New repository secret**。工作流读取下表六项，名称必须一致。
私钥、导出密码直接存入 Secrets，不要发到 Issue、PR 或提交到 Git。

| 名称 | 填什么 | 获取方式 |
|---|---|---|
| `APPLE_CERTIFICATE` | **含私钥**的 Developer ID Application `.p12` 的 Base64 文本 | 按下文导出，再运行 `openssl base64 -A -in Macrun-DeveloperID.p12 -out certificate-base64.txt`；填输出文件全部内容 |
| `APPLE_CERTIFICATE_PASSWORD` | 导出上述 `.p12` 时设置的密码 | 导出时自设并保存；不是 Apple ID 登录密码 |
| `APPLE_SIGNING_IDENTITY` | 完整身份名称，如 `Developer ID Application: Your Name (TEAMID)` | 在持有证书的 Mac 运行 `security find-identity -v -p codesigning`，复制对应身份的引号内文本，不含引号和序号 |
| `APPLE_API_ISSUER` | 团队 API 的 Issuer ID（UUID） | App Store Connect → Users and Access → Integrations → App Store Connect API → Team Keys，复制 Issuer ID；不是 Team ID |
| `APPLE_API_KEY` | 上述团队密钥的 Key ID | 同一页面该密钥对应的 Key ID；不是 `.p8` 内容 |
| `APPLE_API_PRIVATE_KEY` | 下载的 `AuthKey_XXXX.p8` **完整原文** | 下载后复制整个文件，保留 BEGIN / END 行及换行；不要 Base64 编码 |

### A. 准备签名证书

1. 登录 [Apple Developer Account](https://developer.apple.com/account/)，确认
   **Apple Developer Program** 会员有效。有团队时选择之后持续用于签名的团队。
2. 在 Mac 打开“钥匙串访问 → 登录 → 我的证书”。如果已有有效的
   **Developer ID Application**，展开后应看得到私钥；直接复用此身份。
3. 没有证书时，由 **Account Holder** 创建：钥匙串访问 → 证书助理 →
   从证书颁发机构请求证书，填写邮箱、常用名称，选择“存储到磁盘”生成 CSR。
4. Apple Developer → Certificates, Identifiers & Profiles → Certificates → `+` →
   **Developer ID Application**，上传 CSR，下载 `.cer`。在生成 CSR 的同一台 Mac
   双击安装。不要选择 Apple Development、Apple Distribution 或 Developer ID Installer。
5. 在钥匙串“我的证书”选中证书及关联私钥，右键导出为 `.p12`，设置导出密码。
   只有 `.cer` 或没有私钥的文件不能用于 CI 签名；导不出 `.p12` 时先找回对应私钥。
6. 生成 Base64 文件，填写表中前三项 Secrets。私钥与密码自行安全备份，避免每次发布换团队。

参考：[Apple Developer ID 证书](https://developer.apple.com/help/account/certificates/create-developer-id-certificates)、
[Tauri 签名与凭据说明](https://v2.tauri.app/distribute/sign/macos/)。

### B. 准备公证 API 密钥

1. 登录 [App Store Connect](https://appstoreconnect.apple.com/access/integrations/api)，
   选择与签名证书一致的团队，进入 Users and Access → Integrations → App Store Connect API。
2. 如果未开通 API，由 Account Holder 请求访问；开通后由 Account Holder / Admin
   在 **Team Keys** 创建团队密钥，名称可为 `Macrun Release`，权限选 **Developer**。
3. 复制 Issuer ID、Key ID，下载 `.p8` 私钥，填写表中后三项 Secrets。
   私钥只能下载一次，立即安全备份。本工作流使用 Team Key，不使用 Individual Key。

参考：[Apple API 密钥说明](https://developer.apple.com/help/app-store-connect/get-started/app-store-connect-api/)。
使用这套 API 凭据后，不需要另提供 Apple ID 密码、应用专用密码或 Team ID Secret。
DMG 分发也不需要创建 App Store 商品页。

### C. GitHub 配置与回报

- 有仓库设置权限，能够添加上述六个 Repository Secrets。
- Settings → Actions → General 允许本仓库工作流运行，组织策略允许所用 Actions
  和 macOS/Linux runner。工作流自行申请最终上传所需的 `contents: write`。
- 使用 GitHub 自动生成的 `GITHUB_TOKEN`，不需要准备个人 PAT。
- 当前 runner：`macos-15`（arm64）、`macos-15-intel`（Intel）、
  `ubuntu-24.04`（amd64）、`ubuntu-24.04-arm`（arm64）。确认账户可使用并有可用额度。
- 完成后回复：**六项 Secrets 已配置、Apple 会员有效、首次发布版本号、是否有 Intel Mac
  可协助首次安装验收**。只回配置状态，不回秘密值。没有 Intel Mac 时如实说明，
  该平台真实安装验收保持待完成。

参考：[GitHub Secrets 设置](https://docs.github.com/en/actions/how-tos/write-workflows/choose-what-workflows-do/use-secrets)、
[runner 列表](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)。

## 发布前版本准备

目前版本统一为 `0.2.1`。发布新版本时先更新以下字段，并把修改合入 main：

- `Cargo.toml`、`desktop/src-tauri/Cargo.toml` 中本项目包版本。
- `desktop/package.json`、`desktop/src-tauri/tauri.conf.json` 中版本。
- `Cargo.lock`、`desktop/src-tauri/Cargo.lock` 的本项目包版本；
  `desktop/package-lock.json` 顶层和根 package 的版本。

运行 `python3 scripts/release/release.py check --tag vX.Y.Z` 检查一致性。
随后在 main 的该提交打标签并推送，再从 Actions 手动运行。标签必须包含本发布实现，
旧标签不会自动获得新工作流脚本。签名身份和 bundle ID `dev.macrun.desktop` 保持稳定。

## 流水线与重试

工作流锁定标签提交，校验它属于 main，按标签串行执行。分别测试和构建两个 macOS
与两个 Linux 架构。macOS 内置 worker 和主程序分别检查架构，Tauri 完成应用签名，
脚本完成应用公证／staple、DMG 创建／签名／公证／staple，以及挂载后验证。
Linux 在 Debian 12 构建，真实 `.deb` 安装测试同时覆盖 Debian 12、Ubuntu 24.04。

仅所有任务成功后才创建草稿、上传六个安装文件、`release-manifest.json` 和
`SHA256SUMS`，下载远端附件核对哈希后按 `publish` 参数决定是否正式发布。
因此早期构建失败不会新建 Release；已存在的草稿保留。

草稿包含源提交标记，失败后可重跑同一标签；只替换同一提交的预期附件。
工作流拒绝已发布 Release、无来源标记的手动草稿、不匹配提交及陌生附件，
不会自动删除用户附件。发布后修复请使用新版本号，不移动旧标签。
若首次草稿验收通过，可以直接在 GitHub Release 页面发布**该已验收草稿**，
避免重跑生成另一组尚未人工验收的二进制。

Apple 公证失败时查看 Actions 中 `notary-*` 诊断附件。超时或凭据错误会停止发布；
确认原因后重跑。六项 Secrets 未配置时，本地检查仍可运行；签名发布无法完成。

## 首次上线验收

- Apple Silicon 和 Intel Mac 通过浏览器下载 DMG，拖入 Applications 后启动；
  无需禁用 Gatekeeper 或移除隔离属性。确认配对、worker 启动及真实版本升级。
- 首次打开确认、屏幕录制和辅助功能授权仍由用户完成；CuaDriver 另有权限要求。
- macOS 最低系统声明沿用 12.0，CuaDriver 仍要求 macOS 14+；最低系统兼容性
  需要对应系统实机验证，runner 构建成功不能代替它。
- Linux 容器自动验证安装、同版本重装、卸载和基本通信；真实旧版本升级及现场
  systemd 配置仍需按目标部署环境验收。参见 [Linux 安装说明](linux-install.md)。
- 本项目原有桌面产品验收清单也应完成。应用内自动更新不属于本流水线范围。
