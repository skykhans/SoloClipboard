# 网络问题解决方案

## 问题描述

编译 Solo Clipboard 时遇到 GPUI 依赖下载失败：
```
failed to send request: 操作超时
unable to update https://github.com/zed-industries/zed?branch=main
```

这是因为 GPUI 框架依赖 Zed 编辑器的完整源码仓库（约 2-3 GB），网络不稳定时很难下载。

## 解决方案

### 方案 1：使用 VPN 或代理（推荐 ⭐）

如果你有 VPN 或网络代理：

1. 启动 VPN/代理工具
2. 配置 Git 使用代理：
```bash
# HTTP 代理
git config --global http.proxy http://127.0.0.1:7890
git config --global https.proxy http://127.0.0.1:7890

# SOCKS5 代理
git config --global http.proxy socks5://127.0.0.1:7890
git config --global https.proxy socks5://127.0.0.1:7890
```
3. 重新编译：
```bash
cargo clean
cargo build --release
```

### 方案 2：手动下载 Zed 仓库

1. **使用浏览器或下载工具**（迅雷、IDM 等）下载 Zed 仓库：
   - 方式 A：访问 https://github.com/zed-industries/zed/archive/refs/heads/main.zip
   - 方式 B：使用 GitHub 镜像站（如 ghproxy.com）下载

2. **解压到本地**：
   ```
   C:\Test\zed\
   ```

3. **修改 Cargo.toml 使用本地路径**：
   ```toml
   [dependencies]
   gpui = { path = "C:/Test/zed/crates/gpui" }
   # 其他依赖保持不变
   ```

4. **重新编译**：
   ```bash
   cargo build --release
   ```

### 方案 3：使用 Git 镜像站

尝试配置 Git 使用国内镜像：

```bash
# 清除之前的缓存
rm -rf ~/.cargo/git/db/zed-*
rm -rf ~/.cargo/git/checkouts/zed-*

# 配置镜像（选择一个）
# 镜像 1
git config --global url."https://hub.fastgit.xyz/".insteadOf "https://github.com/"

# 镜像 2
git config --global url."https://gitclone.com/github.com/".insteadOf "https://github.com/"

# 重新编译
cargo build --release
```

### 方案 4：分段克隆 Zed 仓库

如果网络时断时续，可以尝试浅克隆：

```bash
cd C:\Test
git clone --depth=1 --single-branch --branch main https://github.com/zed-industries/zed.git
```

然后修改 `Cargo.toml` 使用本地路径（参考方案 2）。

### 方案 5：使用更稳定的网络时段

有时深夜或清晨网络状况会好一些，可以尝试在不同时段编译。

## 验证编译是否成功

编译成功后会看到：

```bash
Finished `dev` profile [unoptimized + debuginfo] target(s) in X.XXs
```

可执行文件位置：
- Debug 版本：`target/debug/solo_clipboard.exe`
- Release 版本：`target/release/solo_clipboard.exe`

## 运行程序

```bash
# 开发版本
cargo run

# 或直接运行编译好的可执行文件
./target/release/solo_clipboard.exe
```

## 常见问题

### Q: 编译需要多长时间？
A: 首次编译 GPUI 项目通常需要 15-30 分钟，具体取决于 CPU 性能。

### Q: 如何查看编译进度？
A: Cargo 会显示正在编译的包名称和数量，例如 "Compiling gpui v0.1.0"。

### Q: 如何取消代理设置？
A: 
```bash
git config --global --unset http.proxy
git config --global --unset https.proxy
git config --global --unset url."https://xxx".insteadOf
```

## 联系支持

如果以上方案都无法解决问题，可以考虑：
1. 在有更好网络环境的地方（如公司、学校）尝试编译
2. 使用云服务器编译后下载可执行文件
3. 联系我寻求其他替代方案

---

**当前状态**：已配置 Cargo 使用更长的超时时间（600秒），配置文件位于 `~/.cargo/config.toml`。
