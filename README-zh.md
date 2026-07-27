# Beacon

[English](README.md) · **中文**

**Beacon 是一款面向 iPhone 与 iPad 的 SSH / Mosh / tmux 终端客户端**，专为需要在远程
服务器上运行 AI 编码 Agent（如 Claude Code 等 CLI 工具）的开发者打造——让你把一个随手可用
的 shell、以及对 Agent 运行状态的掌控，装进口袋里。

它最具特色的功能是 **Vibe Island**：一个实时活动（Live Activity），把远程 Agent 会话的
状态呈现在锁屏和灵动岛上，无需让 App 保持在前台，就能盯着一次耗时的构建或 Agent 运行。

Beacon 使用 SwiftUI 编写，面向 iOS 18，通过
[XcodeGen](https://github.com/yonaskolb/XcodeGen) 生成工程——`project.yml` 是唯一的
事实来源，`Beacon.xcodeproj` 由它生成。

## 功能特性

- **完全免费。** 没有订阅、没有付费墙、没有内购——所有功能对所有人开放。
- **支持漫游的 Mosh。** 会话可在网络切换（Wi-Fi ↔ 蜂窝）和 IP 变化时存活，自动重连而
  不会断开你的 shell。
- **tmux 集成。** 可附着到已有的 tmux 会话，原生切换窗口和面板——App 理解你的会话布局，
  而不是把它当成一段原始回滚缓冲。
- **Vibe Island 实时活动。** 在锁屏和灵动岛上监控远程 Agent 或长时间运行的命令——延迟、
  会话、Agent 状态一目了然。
- **4 套可切换的 App 主题。** Signal Room、Beacon Classic、Terminal Green、Amber
  Console——每套主题都把一种强调色与匹配的备用 App 图标配成一组。终端本身还内置多套配色
  方案（Dracula、Nord、Solarized Dark、Monokai、Tokyo Night 等）。
- **内置 SSH。** 基于 [Citadel](https://github.com/orlandos-nl/Citadel) 的纯 Swift
  SSH 实现，密钥存于 iOS Keychain，用 Face ID 解锁。
- **能干真正活儿的终端。** 基于 [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm)
  的终端模拟（Beacon 使用了一个小幅 fork），带有 `esc`/`ctrl`/`alt`/方向键的自定义快捷
  键栏、输入法/拼音候选合成支持，以及可点击的链接。

## 界面截图

| 服务器列表 | 终端 | tmux 面板 | Vibe Island |
|---|---|---|---|
| ![服务器列表](design-audit/swiftui/02-home.png) | ![终端会话](design-audit/swiftui/04-terminal.png) | ![tmux 面板](design-audit/swiftui/06-tmux-panes.png) | ![实时活动](design-audit/swiftui/10-live-activity.png) |

## 快速开始

### 环境要求

- 装有 Xcode 26 的 macOS
- [XcodeGen](https://github.com/yonaskolb/XcodeGen)：`brew install xcodegen`
- iOS 18 目标（真机或模拟器）

### 构建与运行

```bash
# 1. 克隆仓库
git clone <repository-url>
cd beacon

# 2. 由 project.yml 生成 Xcode 工程
xcodegen generate

# 3. 打开生成的工程
open Beacon.xcodeproj
```

在 Xcode 中选择 **Beacon** scheme，选一个模拟器或你的设备，按 **⌘R** 运行。App 的 bundle
identifier 是 `com.cluas.beacon`（Vibe Island 小组件扩展为 `com.cluas.beacon.island`）。
Swift Package 依赖——SwiftTerm 和 Citadel——会在首次构建时自动解析。

### 安装到你自己的 iPhone

你**不需要**付费的 Apple 开发者账号。一个免费 Apple ID 就足以把 Beacon 侧载到自己的设备
上。这里有一份手把手的指南，涵盖签名、真机安装，以及用 AltStore / SideStore 自动续签的可
选方案：

➡️ **[用免费 Apple ID 安装到 iPhone](docs/install-free-account.md)**

## 架构

想更深入地了解代码结构——各项 service、SwiftUI 布局，以及 Vibe Island 的接线方式——请参阅
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。
