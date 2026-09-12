# Moshpit

[English](README.md) · **中文**

**Moshpit 是 iPhone 和 iPad 上的 SSH / Mosh / tmux 终端，给在远程机器上跑 coding agent
的人用。** Claude Code 或 Codex 在服务器上继续干活；需要你拍板时，锁屏上来一条通知，点一下
就落在提问的那个窗格里。其余时间，灵动岛上看得见谁在干活、谁在等你、等了多久。

[App Store](https://apps.apple.com/app/id6799896801) ·
[官网](https://moshpit.cluas.eu.org/zh) ·
[文档](https://moshpit.cluas.eu.org/zh/docs) ·
[Issues](https://github.com/Cluas/moshpit/issues)

## 截图

| 一眼看全 agent | agent 的窗格 | tmux 窗口 | 锁屏通知 | 九个图标 |
|---|---|---|---|---|
| ![首页的 Agents 区：谁需要你、谁在干活](docs/assets/agents.png) | ![终端里的一个 Claude Code 窗格](docs/assets/agent-terminal.png) | ![tmux 窗口列表，每一行带着自己 agent 的状态](docs/assets/tmux-windows.png) | ![锁屏上一条来自主机的加密通知](docs/assets/lock-screen.png) | ![图标库里的九个 Liquid Glass 图标](docs/assets/icons.png) |

## 能做什么

**首先是一个真正的终端**

- SSH 和 Mosh。Wi-Fi 与 5G 之间切换、手机息屏、酒店的烂网络，会话都断不了。
- tmux 控制模式：原生渲染，左右滑动切换窗口和窗格，能新建、改名、关闭。不是把一块桌面硬塞进手机屏。
- 支持 [herdr](https://github.com/herdrdev/herdr)：为 coding agent 设计的多路复用器，一次只渲染一个满宽窗格。
- iPad：左边是主机，右边是终端，tmux 的选择器以浮窗弹出。外接键盘支持 ⌘K、⌘1–⌘9、⌘W。
- 自定义主题、七种终端字体、九个 Liquid Glass 图标，esc / ctrl / alt / 方向键的快捷键栏，输入法组合输入，可点击的链接，端侧听写。

**为 agent 工作而生**

- 一眼看全每个 agent：谁需要你、谁在干活、干了多久、谁闲着。
- 「需要你」的通知直接落在锁屏上，端到端加密，App 没开也照样送到。点一下，就在提问的那个窗格里。
- 在手机上给 agent 开一个新的 git worktree：选仓库、起分支名，顺手把第一句提示词也交给它。
- 从任何 App 分享一张图，直接进到 agent 的窗格。
- agent 状态在 herdr 上零配置；在 tmux 上装一行 hooks 即可。

**光明正大**

- 一次性买断。没有订阅，没有账号，没有功能分层。
- 不收集任何数据：没有统计，没有追踪。密钥留在 iOS 钥匙串里，流量只去你自己的服务器，外加一台只看得到密文的推送中转服务器。
- Moshpit 不会悄悄往你的主机装东西，不会背着你开会话，传输降级时会明说，而不是默默切换。

任何 SSH 服务器都能用。Mosh、tmux、herdr 都是可选的，安装指引内置在 App 里。

## 从源码构建

需要装有 Xcode 26 的 macOS（App 面向 iOS 18，但要用 iOS 26 SDK 编译）、
[XcodeGen](https://github.com/yonaskolb/XcodeGen)（`brew install xcodegen`），以及一台
iOS 18 或更新的 iPhone / iPad 模拟器或真机。

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # project.yml 是唯一真源，.xcodeproj 由它生成
open Moshpit.xcodeproj
```

选 **Moshpit** scheme 和一台模拟器，按 ⌘R。Swift 包（SwiftTerm 的 fork、Citadel、WhisperKit）
首次构建时自动解析。真机构建要在 `Signing.xcconfig` 里填你的 Team ID，文件里的注释和
[docs/install-free-account.md](docs/install-free-account.md) 都有步骤，包括免费 Apple ID 的走法。

按 CI 的方式跑测试：

```sh
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' CODE_SIGNING_ALLOWED=NO
```

少数几组测试在无签名构建下会跳过（它们需要 App Group 容器），键盘快捷键那组只在 iPad
模拟器上跑；日志里会逐个写明。

## 仓库结构

| 路径 | 内容 |
|---|---|
| `Moshpit/` | App 本体：SwiftUI 界面，SSH / Mosh / tmux / herdr 服务，推送配对，语音输入 |
| `Extensions/` | 三个 App 扩展：`MoshpitIsland/`（实时活动与锁屏小组件，Vibe Island）、`MoshpitPush/`（通知服务扩展，在设备上解开推送）、`MoshpitShare/`（分享扩展，把一张图交给某个 agent 的窗格） |
| `Packages/MoshpitKit/` | App 和三个扩展共用的本地 Swift package：日志、实时活动载荷、推送信封与配对存储 |
| `Tests/` | `MoshpitTests/` 单元测试，`MoshpitUITests/` UI 测试 |
| `push-relay/` | 主机与 APNs 之间的无状态 Go 中转服务器，部署清单在它的 `deploy/` 下 |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md)、[PUSH.md](docs/PUSH.md)（推送协议全流程）、[PATCHES.md](docs/PATCHES.md)（SwiftTerm fork 改了什么）、设计笔记 |
| `scripts/` | `host/` 装到开发机上的 shell 脚本，`gen/` 生成器，`verify/` 端到端验证，`capture/` 截图流程，`spikes/` 实验，`release/` 归档打包 |

App 默认连维护者的中转服务器；[docs/PUSH.md](docs/PUSH.md) 写明了中转服务器能看到什么、看不到什么，以及怎么自己搭一台。

## 参与贡献

先读 [CONTRIBUTING.md](CONTRIBUTING.md)（xcodegen 优先的工作流和代码风格），涉及安全问题请先看
[SECURITY.md](SECURITY.md)。所有人都遵守[行为准则](CODE_OF_CONDUCT.md)。

## 许可证

Moshpit 是自由软件，以 [GNU 通用公共许可证第 3 版](LICENSE)（GPL-3.0-only）发布。你可以在该许可证的
条款下构建、修改和再分发。App Store 上的版本由持有版权的维护者发布。第三方组件保留各自的许可证，
见 [NOTICES.md](NOTICES.md)。
