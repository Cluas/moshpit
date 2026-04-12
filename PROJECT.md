# Vosh — iOS 移动终端客户端

> Vibe + Mosh = Vosh。一款专为远程开发和 AI 编程工作流设计的 iOS SSH/Mosh 终端应用。
> 更实惠的价格，支持买断制。灵动岛实时显示 AI Agent 状态。

## 项目目标

打造一个 iOS 终端客户端，对标 [Moshi](https://getmoshi.app) 和 [Vibe Island](https://vibeisland.app/)，让开发者可以在 iPhone/iPad 上通过 SSH/Mosh 连接远程服务器，运行 Claude Code 等 AI 编程工具，随时随地掌控开发流程。通过灵动岛（Dynamic Island）实时监控 AI Agent 状态，无需切回 App 即可掌握进度、审批操作。

**商业定位：** Moshi 采用订阅制收费，我们提供两种更友好的定价方案：
- 更低价格的订阅方案
- 一次性买断制（终身使用）

## 商业模式

| 方案 | 内容 | 定价参考 |
|------|------|----------|
| 免费版 | SSH 连接（限 2 个配置）、基础终端、基础键盘 | 免费 |
| Pro 订阅 | 无限连接、Mosh、tmux 集成、语音输入、全部主题、推送通知、灵动岛、SFTP、端口转发 | ¥12/月 或 ¥98/年 |
| Pro 买断 | 同 Pro 订阅全部功能，终身使用 | ¥198 一次性 |

技术实现：StoreKit 2（已集成 StoreKit Testing 配置文件）

## 功能总览与开发状态

### P0 — MVP

| 功能 | 状态 | 文件 |
|------|------|------|
| SSH 连接管理（密码/Key 认证） | ✅ 已实现 | `SSHService.swift` |
| Keychain 存储 + Face ID 解锁 | ✅ 已实现 | `KeychainService.swift` |
| 连接配置 CRUD + 持久化 | ✅ 已实现 | `ConnectionStore.swift`, `ServerConnection.swift` |
| 免费版限 2 个连接，Pro 无限制 | ✅ 已实现 | `ConnectionListView.swift` |
| SwiftTerm 终端模拟器 | ✅ 已实现 | `SwiftTerminalView.swift` |
| 5 套终端主题（Dracula/Nord/Solarized/Monokai/TokyoNight） | ✅ 已实现 | `TerminalTheme.swift` |
| 可调节字体大小 | ✅ 已实现 | `AppSettings.swift`, `SettingsView.swift` |
| 自定义终端键盘（Ctrl/Esc/Tab/方向键/快捷键） | ✅ 已实现 | `TerminalKeyboardBar.swift` |
| StoreKit 2 内购（月付/年付/买断） | ✅ 已实现 | `StoreManager.swift`, `PaywallView.swift` |
| StoreKit 测试配置 | ✅ 已实现 | `StoreKit.storekit` |

### P1 — 增强功能

| 功能 | 状态 | 文件 |
|------|------|------|
| Mosh 协议（SSH bootstrap + UDP + 自动重连） | ✅ 已实现 | `MoshService.swift` |
| tmux 深度集成（窗口/面板管理、手势切换） | ✅ 已实现 | `TmuxManager.swift`, `TmuxBarView.swift` |
| 语音输入（Apple Speech Framework，设备端识别） | ✅ 已实现 | `SpeechService.swift`, `VoiceInputButton.swift` |

### P2 — 高级功能

| 功能 | 状态 | 文件 |
|------|------|------|
| 推送通知（本地 + 远程，交互式通知） | ✅ 已实现 | `NotificationService.swift` |
| 服务端推送脚本 | ✅ 已实现 | `scripts/vosh-notify.sh` |
| iPad 适配（NavigationSplitView 分栏布局） | ✅ 已实现 | `SidebarView.swift` |
| SSH 端口转发 | ✅ 已实现 | `PortForwardingService.swift`, `PortForwardingView.swift` |
| SFTP 文件浏览与传输 | ✅ 已实现 | `SFTPService.swift`, `SFTPBrowserView.swift` |
| 终端内容搜索 | ✅ 已实现 | `TerminalSearchBar.swift` |
| 会话录制与回放 | ✅ 已实现 | `SessionRecorder.swift`, `SessionRecordingsView.swift` |

### P3 — 灵动岛 / Dynamic Island

| 功能 | 状态 | 文件 |
|------|------|------|
| Live Activity 数据模型 | ✅ 已实现 | `VoshActivityAttributes.swift` |
| Dynamic Island UI（compact/expanded/minimal） | ✅ 已实现 | `VoshLiveActivity.swift` |
| Lock Screen Live Activity | ✅ 已实现 | `VoshLiveActivity.swift` |
| Live Activity 生命周期管理 | ✅ 已实现 | `LiveActivityService.swift` |
| AI Agent 输出智能解析 | ✅ 已实现 | `AgentOutputParser.swift` |
| Agent 审批按钮（Approve/Deny） | ✅ 已实现 | `VoshLiveActivity.swift` |
| Widget Extension target | ✅ 已实现 | `VoshWidget/` |

**灵动岛显示内容：**
- 连接状态（connected/running/waiting/completed/disconnected/reconnecting）
- AI Agent 状态（thinking/coding/asking/toolUse/error）
- 当前运行命令 + 计时
- 文件变更数 + CPU 占用
- 最后一行终端输出
- Agent 需要输入时提供 Approve/Deny 交互按钮

**AI Agent 检测支持：** Claude Code、Copilot、Cursor 等主流 AI 编码工具

## 技术架构

```
┌──────────────────────────────────────────────────┐
│                   SwiftUI                        │
│          (Views / Navigation / Theme)            │
├──────────────────────────────────────────────────┤
│               ViewModel Layer                    │
│  ConnectionVM │ TerminalVM │ StoreManager        │
├──────────────────────────────────────────────────┤
│              Core Services                       │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│  │SSH Client│ │Mosh      │ │ SwiftTerm        │ │
│  │          │ │Client    │ │ (Terminal Emu)    │ │
│  └──────────┘ └──────────┘ └──────────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│  │Keychain  │ │SFTP      │ │ Port Forwarding  │ │
│  │+ Face ID │ │Browser   │ │ (SSH Tunnel)     │ │
│  └──────────┘ └──────────┘ └──────────────────┘ │
│  ┌──────────┐ ┌──────────┐ ┌──────────────────┐ │
│  │tmux      │ │Speech    │ │ Session Recorder │ │
│  │Manager   │ │Service   │ │                  │ │
│  └──────────┘ └──────────┘ └──────────────────┘ │
├──────────────────────────────────────────────────┤
│            Live Activity / Dynamic Island        │
│  ┌──────────────────┐ ┌────────────────────────┐ │
│  │LiveActivityService│ │AgentOutputParser       │ │
│  │(ActivityKit)      │ │(Claude/Copilot/Cursor) │ │
│  └──────────────────┘ └────────────────────────┘ │
├──────────────────────────────────────────────────┤
│            Platform Services                     │
│  Keychain │ Face ID │ APNs │ StoreKit 2         │
│  Speech   │ ActivityKit │ WidgetKit             │
└──────────────────────────────────────────────────┘
```

## 技术选型

| 模块 | 方案 | 说明 |
|------|------|------|
| UI 框架 | SwiftUI | iOS 17.0+ 最低支持版本 |
| SSH | [Citadel](https://github.com/orlandos-nl/Citadel) v0.12 | 纯 Swift SSH2，基于 SwiftNIO |
| Mosh | [mosh](https://github.com/mobile-shell/mosh) 源码交叉编译 | UDP + SSP 协议 |
| 终端模拟 | [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) v1.13 | 纯 Swift 的 xterm 兼容终端 |
| 密钥存储 | iOS Keychain + LocalAuthentication | Face ID / Touch ID 保护 |
| 内购 | StoreKit 2 | 订阅 + 买断，含 StoreKit Testing 配置 |
| 推送通知 | APNs + 本地通知 + 交互式通知 | 3 种通知类型 |
| 灵动岛 | ActivityKit + WidgetKit | Dynamic Island + Lock Screen |
| 语音识别 | Apple Speech Framework | 设备端识别，保护隐私 |
| 项目生成 | [XcodeGen](https://github.com/yonaskolb/XcodeGen) | 通过 `project.yml` 生成 .xcodeproj |
| 包管理 | Swift Package Manager | 统一依赖管理 |

## 项目结构

```
moshi/
├── project.yml                        # XcodeGen 项目定义
├── PROJECT.md                         # 本文档
├── Vosh.xcodeproj/                    # 生成的 Xcode 项目
│
├── Vosh/                              # 主 App
│   ├── App/
│   │   ├── VoshApp.swift              # App 入口 + AppDelegate
│   │   ├── Info.plist                 # 生成的 Info.plist
│   │   ├── StoreKit.storekit          # StoreKit 测试配置
│   │   └── Assets.xcassets/           # 图标 + 颜色
│   │
│   ├── Models/
│   │   ├── ServerConnection.swift     # 连接配置模型
│   │   ├── ConnectionStore.swift      # 连接持久化（UserDefaults）
│   │   ├── AppSettings.swift          # 全局设置（@AppStorage）
│   │   ├── TerminalTheme.swift        # 5 套主题 + Color hex 扩展
│   │   └── VoshActivityAttributes.swift # Live Activity 数据模型（共享）
│   │
│   ├── Services/
│   │   ├── SSHService.swift           # SSH 连接管理
│   │   ├── MoshService.swift          # Mosh 协议（SSH bootstrap + UDP）
│   │   ├── KeychainService.swift      # Keychain 存储 + Face ID
│   │   ├── StoreManager.swift         # StoreKit 2 内购管理
│   │   ├── TmuxManager.swift          # tmux 窗口/面板/会话管理
│   │   ├── SpeechService.swift        # 语音识别
│   │   ├── NotificationService.swift  # 推送通知（3 种类型 + 交互）
│   │   ├── LiveActivityService.swift  # Live Activity 生命周期
│   │   ├── AgentOutputParser.swift    # AI Agent 输出模式识别
│   │   ├── PortForwardingService.swift # SSH 端口转发（Citadel DirectTCPIP）
│   │   ├── SFTPService.swift          # SFTP 文件浏览（Citadel SFTP）
│   │   ├── SessionRecorder.swift      # 会话录制
│   │   ├── HostKeyValidator.swift     # TOFU 主机密钥校验
│   │   ├── SSHKeyManager.swift        # SSH 密钥生成/导入
│   │   └── DeepLinkHandler.swift      # vosh:// URL Scheme 处理
│   │
│   ├── ViewModels/
│   │   ├── ConnectionViewModel.swift  # 连接表单逻辑
│   │   └── TerminalViewModel.swift    # 终端会话 + Live Activity 集成
│   │
│   └── Views/
│       ├── ConnectionList/
│       │   ├── ConnectionListView.swift   # 连接列表 + Pro 限制
│       │   └── ConnectionFormView.swift   # 添加/编辑连接
│       ├── Terminal/
│       │   ├── TerminalContainerView.swift # 终端容器（集成所有功能）
│       │   ├── SwiftTerminalView.swift    # SwiftTerm UIKit 桥接
│       │   ├── TerminalKeyboardBar.swift  # 自定义功能键栏
│       │   ├── TerminalSearchBar.swift    # 终端搜索
│       │   ├── TmuxBarView.swift          # tmux 窗口标签栏
│       │   ├── VoiceInputButton.swift     # 语音输入按钮
│       │   ├── PortForwardingView.swift   # 端口转发管理
│       │   ├── SessionRecordingsView.swift # 录制回放列表
│       │   ├── SessionPlayerView.swift    # 回放播放器（倍速/跳转）
│       │   └── TerminalGestureHandler.swift # 手势（缩放/粘贴/滑动）
│       ├── Settings/
│       │   ├── SettingsView.swift         # 设置（字体/主题/预览）
│       │   └── KeyManagementView.swift    # SSH 密钥管理（生成/导入）
│       ├── Store/
│       │   └── PaywallView.swift          # 付费墙（功能列表 + 定价）
│       ├── SFTP/
│       │   └── SFTPBrowserView.swift      # 文件浏览器
│       └── iPad/
│           └── SidebarView.swift          # iPad 分栏布局
│
├── VoshWidget/                        # Widget Extension
│   ├── VoshWidgetBundle.swift         # Widget 入口
│   ├── VoshLiveActivity.swift         # Dynamic Island + Lock Screen UI
│   ├── VoshStatusWidget.swift         # Home Screen 桌面小组件
│   └── Info.plist                     # 生成的 Info.plist
│
├── scripts/
│   └── vosh-notify.sh                 # 服务端推送辅助脚本
│
└── Vosh/Resources/
    ├── en.lproj/Localizable.strings   # 英文
    └── zh-Hans.lproj/Localizable.strings # 中文
```

## 待完善项（TODO）

### 已完成

- [x] **真实 SSH 连接** — Citadel 纯 Swift SSH2，支持密码/RSA Key 认证、PTY 交互式 Shell
- [x] **SFTP 原生实现** — Citadel SFTP subsystem，目录浏览、文件上传下载、创建/删除/重命名
- [x] **端口转发实现** — Citadel DirectTCPIP channel
- [x] **URL Scheme 处理** — `vosh://` 注册 + DeepLinkHandler，灵动岛 Approve/Deny 跳转
- [x] **Host Key 安全校验** — TOFU（Trust-On-First-Use）策略，首次连接提示信任，密钥变更警告
- [x] **SSH Key 管理** — 生成 Ed25519/ECDSA 密钥、从文件导入、公钥复制
- [x] **会话回放播放器** — SessionPlayerView，支持播放/暂停/倍速/跳转
- [x] **桌面 Widget** — Home Screen 小组件显示服务器状态（small/medium 两种尺寸）
- [x] **国际化** — 中英文 Localizable.strings，覆盖全部 UI 文案
- [x] **终端手势增强** — 双指缩放字体、双击粘贴、左右滑动切换 tmux 窗口

### 需要 Apple Developer 账号

- [ ] **APNs 远程推送** — 需要开发者账号配置推送证书 + 服务端推送接口
- [ ] **StoreKit 真机测试** — StoreKit Testing 配置已就绪，需在真机验证购买流程
- [ ] **TestFlight 分发** — 内测分发

### 已完成（本轮）

- [x] **Mosh C++ 交叉编译脚本** — `scripts/build-mosh-ios.sh`，自动下载 + 编译 mosh/protobuf for iOS arm64
- [x] **AgentOutputParser 调优** — 基于 Claude Code 真实输出格式（Edit/Read/Bash/Glob 等工具名、Braille spinner、权限提示）优化匹配规则
- [x] **Live Activity Preview** — Widget Extension 内 SwiftUI Preview，模拟 Running/Waiting/Idle 三种状态
- [x] **App Icon** — 像素风终端窗口图标（1024x1024），`scripts/generate-app-icon.swift` 代码生成
- [x] **Whisper 本地模型** — 集成 WhisperKit（tiny/base/small），支持离线设备端语音识别
- [x] **App Group 共享** — `SharedDataStore` + entitlements，Widget 和主 App 通过 `group.com.cluas.vosh` 共享连接数据

### 仅需 Apple Developer 账号

- [ ] **APNs 远程推送** — 需开发者账号配置推送证书
- [ ] **StoreKit 真机验证** — 需 Sandbox 环境测试真实购买
- [ ] **TestFlight 分发** — 内测分发

## 竞品对比
//
| App | 特点 | 灵动岛 | 价格 |
|-----|------|--------|------|
| [Moshi](https://getmoshi.app) | Mosh 优先、AI 工作流、语音输入 | 无 | 订阅制 |
| [Vibe Island](https://vibeisland.app) | macOS notch 显示 Agent 状态、审批操作 | macOS only | $19.99 买断 |
| [Blink Shell](https://blink.sh) | 开源、Mosh、成熟稳定 | 无 | 订阅制 |
| [Termius](https://termius.com) | 跨平台、SFTP、团队功能 | 无 | 订阅制 |
| [Prompt](https://panic.com/prompt/) | Panic 出品、设计精美 | 无 | 买断 |
| **Vosh** | AI 工作流 + 灵动岛 Agent 监控 + 买断制 | **iOS 灵动岛** | 订阅 + 买断 |

## 差异化竞争力

1. **灵动岛 AI Agent 监控** — 唯一在 iOS 灵动岛实时显示 Claude Code/Copilot 状态的终端 App，支持直接审批操作
2. **买断制可选** — 大部分竞品只提供订阅，我们支持一次性买断
3. **更低的订阅价格** — 以更实惠的定价争取价格敏感用户
4. **Vibe Coding 工作流优先** — 针对 AI Agent 深度优化（状态检测、交互通知、Agent 输出解析）
5. **中文社区友好** — 完善的中文本地化和文档支持

## 构建与运行

```bash
# 安装 XcodeGen（仅首次）
brew install xcodegen

# 生成 Xcode 项目
xcodegen generate

# 用 Xcode 打开
open Vosh.xcodeproj

# 命令行编译
xcodebuild -project Vosh.xcodeproj -scheme Vosh \
  -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' build
```

## 环境要求

- Xcode 16+
- iOS 17.0+ / iPadOS 17.0+（灵动岛需 iPhone 14 Pro 及以上）
- Swift 5.9+
- macOS 14+（开发环境）
