# Vosh — iOS SSH/Mosh Terminal

> Vibe + Mosh = Vosh. AI 编程工作流优先的 iOS 终端客户端。

## 快速开始

### 环境要求

- macOS 14+
- Xcode 16+
- [XcodeGen](https://github.com/yonaskolb/XcodeGen): `brew install xcodegen`

### 构建 & 运行

```bash
# 1. 生成 Xcode 项目
xcodegen generate

# 2. 命令行编译
xcodebuild -project Vosh.xcodeproj -scheme Vosh \
  -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' build

# 3. 或者用 Xcode 打开
open Vosh.xcodeproj
# 选择 iPhone 17 Pro 模拟器，Cmd+R 运行
```

### 模拟器测试

```bash
# 启动模拟器
xcrun simctl boot CE0FEF85-AAE0-48BF-9FCB-6D56AAEEA898
open -a Simulator

# 安装 & 启动 app
APP=$(find ~/Library/Developer/Xcode/DerivedData/Vosh-*/Build/Products/Debug-iphonesimulator -name "Vosh.app" -type d | head -1)
xcrun simctl install booted "$APP"
xcrun simctl launch booted com.cluas.vosh

# 截图
xcrun simctl io booted screenshot /tmp/vosh.png
```

### 一键编译部署

```bash
xcodegen generate && \
xcodebuild -project Vosh.xcodeproj -scheme Vosh \
  -sdk iphonesimulator \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build && \
xcrun simctl install booted \
  "$(find ~/Library/Developer/Xcode/DerivedData/Vosh-*/Build/Products/Debug-iphonesimulator -name 'Vosh.app' -type d | head -1)" && \
xcrun simctl launch booted com.cluas.vosh
```

## 测试流程

### SSH 连接测试

1. 启动 App → 点 ⊕ 添加服务器
2. 填入：Host / Port(22) / Username / Password
3. Protocol 选 **SSH**
4. 点 Save → 点连接卡片
5. 应看到：连接动画 → tmux 检测 → 终端 shell

### Mosh 连接测试

前提：远程服务器已安装 `mosh-server`

1. 添加服务器，Protocol 选 **Mosh**
2. Server Path 填 `mosh-server`（或 `/opt/homebrew/bin/mosh-server`）
3. UDP Ports 保持默认 60001-60999
4. 连接后：SSH bootstrap → mosh-server 启动 → 终端 shell

### tmux Session 测试

前提：远程服务器有运行中的 tmux session

1. 先在服务器上创建 tmux：`tmux new -s dev`
2. 在 Vosh 中连接该服务器
3. 连接动画完成后应显示 **tmux sessions** 选择页
4. 选择 session → 查看 layout → Attach

### 键盘快捷栏测试

- 终端底部应有横向滚动的快捷键栏
- 包含：Ctrl / Esc / Tab / ↑↓←→ / C-c C-d C-z C-l / | ~ / - _ 等
- 点 Ctrl 切换为高亮（组合键模式），再点任意键发送 Ctrl+X

### 付费墙测试

- Debug 模式下默认解锁 Pro（`StoreManager._debugProOverride = true`）
- 改为 `false` 后可测试免费用户视角
- 免费版限 2 个连接，点 Pro 功能弹付费墙

### StoreKit 测试

1. Xcode → Product → Scheme → Edit Scheme
2. Run → Options → StoreKit Configuration → 选 `StoreKit.storekit`
3. 运行后可测试完整购买流程（Sandbox 环境）

## 项目结构

```
Vosh/           主 App（SwiftUI + Citadel SSH + SwiftTerm）
VoshWidget/     Widget Extension（Dynamic Island + 桌面小组件）
build/          mosh C++ 静态库编译产物
scripts/        构建脚本（mosh 编译、App Icon 生成、服务端推送）
```

详见 [PROJECT.md](PROJECT.md)

## Debug 开关

| 开关 | 位置 | 作用 |
|------|------|------|
| `_debugProOverride` | `StoreManager.swift` | `true` = 解锁全部 Pro 功能 |
| `preferredColorScheme(.dark)` | 各 View | 强制深色模式 |
