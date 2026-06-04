# Quay

Quay 是一个面向 macOS 的 `frpc` 桌面端，基于 **Tauri v2 + React + shadcn/ui** 构建。

它的目标不是重新实现 frp，而是把常用的本地桌面体验补齐：
- 托盘控制
- 启动 / 停止 / 重启 `frpc`
- 简单配置编辑
- 最近日志查看
- macOS 开机启动
- 更适合日常使用的桌面窗口体验

## Features

- **macOS 原生桌面封装**：Tauri v2 打包为 `.app` 与 `.dmg`
- **托盘菜单**：可从托盘打开主窗口、打开设置、启动 / 停止 `frpc`
- **配置编辑**：内置 TOML 配置编辑区
- **日志查看**：直接查看最近运行日志
- **开机启动**：支持应用级自动启动，并可配置启动后是否自动拉起 `frpc`
- **沉浸式窗口**：透明标题栏、保留标准红绿灯、支持顶部拖拽
- **深色 / 浅色主题**：界面跟随应用内切换逻辑

## Tech Stack

- **Desktop shell**: Tauri v2 + Rust
- **Frontend**: React 19 + TypeScript + Vite
- **UI**: shadcn/ui + Radix UI + Tailwind CSS
- **Config parsing**: `smol-toml`

## Project Structure

```text
src/                React UI
src-tauri/          Tauri / Rust host
src-tauri/resources/
  frpc              bundled frpc binary
  frpc.toml         bundled default config
.github/workflows/  GitHub Actions release workflow
```

## Local Development

Install dependencies:

```bash
pnpm install
```

Run the frontend dev server:

```bash
pnpm dev
```

Run the Tauri app in development:

```bash
pnpm tauri dev
```

## Local Build

Build the frontend:

```bash
pnpm build
```

Build the macOS desktop app:

```bash
pnpm tauri build --bundles app,dmg
```

## Release Process

当前仓库已经接入 **GitHub Actions 自动发版**。

### Trigger Rule

当前不是“普通 push commit 自动发版”，而是：

- **push 一个符合 `v*` 的 Git tag 时自动发版**

例如：

```bash
git tag v0.1.0
git push origin v0.1.0
```

### What Happens In CI

触发后，GitHub Actions 会自动：

1. 在 `macos-latest` 上启动工作流
2. 安装 Node.js / pnpm / Rust
3. 执行 `pnpm install --frozen-lockfile --ignore-scripts`
4. 调用 Tauri 构建
5. 自动创建或更新对应的 GitHub Release
6. 上传构建产物

工作流文件：

- `.github/workflows/release.yml`

### Published Assets

当前会上传这些主要产物：

- `Quay_*.dmg` —— 给最终用户下载使用
- `Quay_*.app.tar.gz` —— `.app` bundle 压缩产物，便于排查或手动分发

### Versioning Convention

建议正式版本使用：

- `v0.1.0`
- `v0.1.1`
- `v0.2.0`

测试验证可用：

- `v0.1.0-test1`
- `v0.1.0-verify1`

## Current Release Notes

- 当前发布仅支持 **macOS**
- 当前发布为 **未签名 / 未公证** 版本
- 首次启动时，macOS 仍可能提示 Gatekeeper 安全确认
- Apple 签名 / 公证所需 secrets 仅在 workflow 中预留，尚未启用

## Bundle Targets

当前 Tauri 打包目标为：

- `app`
- `dmg`

## Repository Notes

- 仓库中不应提交个人真实 `frpc.toml`
- 当前 `src-tauri/resources/frpc.toml` 应仅作为默认示例配置
- 当前 `src-tauri/resources/frpc` 作为受控资源随应用一同打包

## Next Possible Improvements

- Apple 签名与 notarization
- 自动检查 `frpc` 二进制版本
- 配置文件语法高亮与更完整的错误提示
- 发布页补充更正式的 changelog 与安装说明
