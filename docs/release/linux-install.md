# Linux 安装

支持 amd64、arm64；验收环境为 Debian 12、Ubuntu 24.04。包在 Debian 12
构建，依赖 glibc，不是静态 musl 程序。其他发行版请先核对动态库兼容性。

从同一个 Release 下载安装包与 `SHA256SUMS`，核对所下载文件的 SHA-256。
所有文件均已下载时可运行 `sha256sum -c SHA256SUMS`。

```sh
# 按 CPU 架构选择文件；示例版本仅供参考
sudo apt install ./macrun_0.2.0_amd64.deb
macrun --version
```

`.deb` 安装到 `/usr/bin/macrun`。命令行、server、worker、MCP 使用同一程序。
安装不会创建身份、修改防火墙或自动启动服务。

使用 `.tar.gz` 时解压后安装：

```sh
mkdir -p "$HOME/.local/bin"
install -m755 ./macrun "$HOME/.local/bin/macrun"
```

确保 `~/.local/bin` 已在 PATH 中，避免它覆盖另一版本的 `/usr/bin/macrun`。

## 服务配置

先以实际服务用户执行 `macrun init --data /absolute/path/to/state`，只在首次部署
初始化；升级时保留原数据和证书。监听地址、服务用户和防火墙由管理员按部署环境设置。

`.deb` 的服务模板在 `/usr/share/doc/macrun/macrun-server.service.example`；
压缩包的模板位于解压目录。复制到工作目录，替换 `REPLACE_USER` 与数据路径。
通过 `.deb` 安装时，把模板中的 `/usr/local/bin/macrun` 改成 `/usr/bin/macrun`；
用户目录安装时使用该目录的绝对路径。确认数据目录归服务用户所有，再执行：

```sh
sudo systemd-analyze verify ./macrun-server.service
sudo install -m644 ./macrun-server.service /etc/systemd/system/macrun-server.service
sudo systemctl daemon-reload
sudo systemctl enable --now macrun-server
```

配置详情见仓库 `docs/operations.md`。包中模板不会直接安装成启用的系统服务。

## 升级与卸载

升级前停止运行中的 server/worker，再安装新包并重新启动；包不会自动重启进程。
同一版本重新安装用 `sudo apt install --reinstall ./文件.deb`。

卸载前停用自己配置的服务：`sudo systemctl disable --now macrun-server`。
随后执行 `sudo apt remove macrun`；手动安装的服务文件和用户数据不会被删除，
需要时由管理员单独处理。压缩包安装则删除安装位置的可执行文件即可。
