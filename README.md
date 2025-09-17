# SmartEDU Downloader CLI

<p align="center">
  <strong>一个基于 Rust 开发的、高效、跨平台的国家中小学智慧教育平台教材下载命令行工具。</strong>
</p>

<p align="center">
    <a href="https://github.com/lss53/smartedu_downloader_cli/actions/workflows/release.yml">
        <img src="https://github.com/lss53/smartedu_downloader_cli/actions/workflows/release.yml/badge.svg" alt="Build Status">
    </a>
    <a href="https://github.com/lss53/smartedu_downloader_cli/releases/latest">
        <img src="https://img.shields.io/github/v/release/lss53/smartedu_downloader_cli" alt="Latest Release">
    </a>
    <a href="https://opensource.org/licenses/MIT">
        <img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT">
    </a>
</p>

---

## ✨ 特性

- **⚡ 高性能**: 基于 Rust 和 Tokio 异步运行时构建，资源占用低，下载速度快。
- **🔗 多任务并发**: 支持设置并发任务数，显著提升批量下载效率。
- **🖥️ 跨平台**: 单个可执行文件，完美支持 Windows, macOS 和 Linux，无需额外依赖。
- **🤖 智能校验**:
    - 下载前检查本地文件，通过 MD5 或文件大小校验，避免重复下载。
    - 下载后自动校验文件完整性，确保文件准确无误。
- **🎨 优秀的用户体验**:
    - 美观的多进度条显示，实时追踪每个下载任务的状态。
    - 彩色日志输出，信息清晰易读。
    - 交互式 Token 获取引导，并支持自动保存，免去重复输入。
- **多种输入方式**: 支持通过单个 URL、Content ID，或从文件进行批量下载。

## 📥 安装与使用

### 1. 下载预编译的可执行文件

最简单的使用方式是直接从 [Releases 页面](https://github.com/lss53/smartedu_downloader_cli/releases/latest) 下载。

1.  访问最新的 Release 页面。
2.  根据您的操作系统，下载对应的文件：
    -   Windows: `smartedu_downloader-windows-x64.exe`
    -   macOS: `smartedu_downloader-macos-x64`
    -   Linux: `smartedu_downloader-linux-x64`
3.  (macOS/Linux 用户) 下载后，请先赋予文件可执行权限：
    ```bash
    chmod +x ./smartedu_downloader-macos-x64
    ```

### 2. 准备 Access Token

本工具需要使用您在[国家中小学智慧教育平台](https://auth.smartedu.cn/uias/login)的 `Access Token` 来进行下载。

首次运行程序时，它会引导您如何获取并输入 Token。Token 会被自动保存在程序目录下的 `.access_token` 文件中，方便后续使用。

### 3. 使用示例

在您的终端（命令行、PowerShell）中运行程序。

#### 下载单个教材

```bash
# 通过 URL
./smartedu_downloader -u "https://basic.smartedu.cn/tchMaterial/detail?contentId=..."

# 通过 Content ID
./smartedu_downloader -c "教材的Content-ID"
```

#### 批量下载 (推荐)
1.  创建一个文本文件，例如 `urls.txt`。
2.  在文件中每行放置一个 URL 或 Content ID。
    ```txt
    # urls.txt
    https://basic.smartedu.cn/tchMaterial/detail?contentId=...
    另一个教材的Content-ID
    ```
3.  运行命令，并指定输出目录：
    ```bash
    # 下载到名为 "教材下载" 的文件夹中
    ./smartedu_downloader -i urls.txt -o ./教材下载/
    ```

#### 查看所有选项
```bash
./smartedu_downloader --help
```

## 🛠️ 从源码编译 (适合开发者)

如果您希望自行修改或编译本项目，请确保您已经安装了 [Rust 工具链](https://rustup.rs/)。

```bash
# 1. 克隆仓库
git clone https://github.com/<你的用户名>/<你的仓库名>.git
cd <你的仓库名>

# 2. 编译 Release 版本
cargo build --release

# 3. 运行
# 编译后的可执行文件位于 ./target/release/ 目录下
./target/release/smartedu_downloader --help
```

## 🤝 贡献

欢迎任何形式的贡献！如果您有好的想法、功能建议或发现了 Bug，请随时提交 [Issues](https://github.com/lss53/smartedu_downloader_cli/issues) 或 [Pull Requests](https://github.com/lss53/smartedu_downloader_cli/pulls)。

## 📝 许可 (License)

本项目采用 [MIT License](https://opensource.org/licenses/MIT) 授权。

---

**免责声明**: 本工具仅供学习和技术研究使用，请勿用于商业用途。所有下载内容的版权归国家中小学智慧教育平台及其相关方所有。

