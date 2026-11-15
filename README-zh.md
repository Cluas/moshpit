# Moshpit

[English](README.md) · **中文**

**Moshpit 是 iPhone 和 iPad 上的 SSH / Mosh / tmux 终端，写给在远程机器上跑编程智能体的人。**
Claude Code 或 Codex 在服务器上接着干活；需要你拍板时，锁屏上来一条通知，点一下就落在提问的那个窗格里。
其余时间，灵动岛上看得见谁在干活、谁在等你、等了多久。

[App Store](https://apps.apple.com/app/id6799896801) ·
[官网](https://moshpit.cluas.eu.org/zh) ·
[文档](https://moshpit.cluas.eu.org/zh/docs) ·
[Issues](https://github.com/Cluas/moshpit/issues)

## 截图

| 一眼看全智能体 | 智能体的窗格 | tmux 窗口 | 锁屏通知 | 九个图标 |
|---|---|---|---|---|
| ![首页的智能体区：谁需要你、谁在干活](docs/assets/agents.png) | ![终端里的一个 Claude Code 窗格](docs/assets/agent-terminal.png) | ![tmux 窗口列表，每一行带着它那个智能体的状态](docs/assets/tmux-windows.png) | ![锁屏上一条来自主机的加密通知](docs/assets/lock-screen.png) | ![图标库里的九个 Liquid Glass 图标](docs/assets/icons.png) |

## 能做什么

**首先是一个真正的终端**

- SSH 和 Mosh，Mosh 带预测回显。Wi-Fi 和 5G 来回切、手机息屏、酒店的烂网络，会话都不断；App 在后台待过一阵回来，连接还活着就直接接着用，不用重连。
- tmux 控制模式：原生渲染，左右滑动切窗口和窗格，能新建、改名、关闭。不是把一块桌面硬塞进手机屏。
- 支持 [herdr](https://github.com/herdrdev/herdr)：专门给编程智能体做的多路复用器，一次只画一个满宽窗格。
- 终端用 Metal 画，内核是我们改过的 wezterm 终端：能显示图片（kitty、iTerm2、sixel），能在历史里搜索，选区带拖动手柄，翻历史时手指一甩还会顺着惯性滑一段。
- iPad：左边主机，右边终端，tmux 的选择器从浮窗弹出。外接键盘支持 ⌘K、⌘1–⌘9、⌘W。
- 自定义主题、七种终端字体、九个 Liquid Glass 图标，esc / ctrl / alt / 方向键的快捷键栏，输入法组合输入，链接点得开，听写在本机完成。
- 每台主机有自己的头像：由它的主机密钥画出来，同一把密钥永远是同一张脸；也可以换成你挑的 emoji。

**为智能体干活而做**

- 一眼看全每个智能体：谁需要你、谁在干活、干了多久、谁闲着。
- 「需要你」的通知直接落在锁屏上，端到端加密，App 没开也照样送到。点一下，就在提问的那个窗格里。
- 智能体在输出里提到的文件，在手机上就能打开：点一下路径，图片直接看、视频直接放、文本直接读，还能翻它所在的文件夹，顺手分享出去。
- 在手机上给智能体开一个新的 git worktree：选仓库、起分支名，顺手把第一句提示词也交给它。
- 从任何 App 分享一张图，直接进到智能体的窗格。
- 一只像素小宠物：在首页从蛋里孵出来，蹲在最需要你的那个智能体的卡片上，跟着它的状态换姿势，灵动岛和实时活动里也有它。打开它以后，每台主机的头像也会变成一只像素小怪，长相同样由主机密钥决定。
- 智能体的状态在 herdr 上不用配置；在 tmux 上在主机上装一行就行。

**光明正大**

- 一次性买断。没有订阅，没有账号，没有功能分层。
- 不收集任何数据：没有统计，没有追踪。密钥留在 iOS 钥匙串里，流量只去你自己的服务器，外加一台只看得到密文的推送转发服务。
- Moshpit 不会悄悄往你的主机装东西，不会背着你开会话；连接降级时会直说，而不是默默换一种方式连。

任何 SSH 服务器都能用。Mosh、tmux、herdr 都是可选的，App 里带着安装指引。

## 怎么搭起来的

Moshpit 大部分是 Rust 写的。核心在 `crates/` 里：SSH（`ssh`，基于 russh）、Mosh 的协议（`mosh`）、负责拨号、重连和回来接着用的会话层（`session`）、tmux 控制模式（`tmux`）和 herdr（`herdr`）、终端模拟器（`term`，底下是 `vendor/wezterm` 里我们改过的 wezterm）、App 要存的东西（`store`）、智能体的状态机（`agents`）、像素小宠物（`pet`），以及主机设置和推送配对（`hostcare`、`install`、`seal`）。界面也是 Rust 写的，用的是 [NSH](https://nsh.dev)，放在 `ui/`。iOS 上只有薄薄一层 Swift，通过一个静态库（`crates/ffi`）把这些接进来：App 本身的外壳、终端视图的 Metal 绘制和文字输入、钥匙串、语音输入，以及 iOS 只让自己人做的那几样（实时活动和小组件、在本机解开推送的通知服务、分享扩展）。安卓版和 iOS 共用同一套核心和界面（`crates/android`、`android/`），还在开发中，没有发布。

## 从源码构建

需要装有 Xcode 27 的 macOS（App 面向 iOS 18）、[XcodeGen](https://github.com/yonaskolb/XcodeGen)
（`brew install xcodegen`），以及用 [rustup](https://rustup.rs) 装的 Rust。仓库在
`rust-toolchain.toml` 里钉死了工具链版本，rustup 第一次用到时会自己装上。

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # project.yml 是唯一的来源，.xcodeproj 由它生成
open Moshpit.xcodeproj
```

选 **Moshpit** scheme 和一台模拟器，按 ⌘R。App target 里的「Build Rust」阶段会跑
`scripts/rust/build-ios.sh`，把 `crates/ffi` 编成给模拟器或真机用的 `libmoshpit_ffi.a`，
第一次会顺手装上需要的 Rust target（`aarch64-apple-ios-sim`、`aarch64-apple-ios`）。
Debug 用 cargo 的 `app-test` 配置，Release 用 `app`。第一次构建要把整套 Rust 编一遍，
得等一会儿，之后就是增量的。Swift 包也在第一次构建时自动拉取。

真机构建要在 `Signing.xcconfig` 里填你的 Team ID，文件里的注释和
[docs/install-free-account.md](docs/install-free-account.md) 一步步讲了怎么填，免费 Apple ID 也行。

### 测试

Rust 这边，和 CI 跑的一样：

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p moshpit-term        # 只测一个 crate
```

iOS App，和 CI 在 Xcode 27 镜像上跑的一样：

```sh
scripts/rust/test-build-ios.sh    # 不编译，只查 Rust 阶段用的配置和链接
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' CODE_SIGNING_ALLOWED=NO
```

有几组测试在没签名的构建里会跳过（它们要用 App Group 容器），有几组只在 iPad 模拟器上跑（外接键盘的快捷键），
还有一些 UI 流程在按现在的界面重写之前也先跳过；日志里会写明是哪些。

安卓版用 `android/ci.sh` 构建和测试，装好 JDK 17 就行，SDK、NDK、Gradle 这些它会自己去拿。

## 仓库结构

| 路径 | 放的是什么 |
|---|---|
| `crates/` | Rust 核心，一件事一个 crate：`ssh`、`mosh`、`session`、`tmux`、`herdr`、`term`、`stream`（窗格的字节在核心和终端视图之间的通道）、`store`、`agents`、`pet`（像素小宠物）、`seal`、`hostcare`、`install`、`app`（各个界面的模型）、`runtime`、`copy`（中文文案守的规矩）、`ffi`（iOS 链接的静态库）、`android`（安卓版的 Rust 那一半），还有两个早期的验证项目 |
| `ui/` | 用 Rust 和 NSH 写的界面，iOS 和安卓共用 |
| `vendor/wezterm/` | 我们改过的 wezterm 终端模拟器，接在 `crates/term` 后面；改了什么见 [docs/WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md) |
| `Moshpit/` | iOS 这一层：App 外壳、每个界面在 Swift 这边的部分、终端视图的 Metal 绘制和文字输入、钥匙串、语音输入（WhisperKit） |
| `Extensions/` | 三个 App 扩展：`MoshpitIsland/`（实时活动和锁屏小组件，Vibe Island）、`MoshpitPush/`（在本机解开推送的通知服务）、`MoshpitShare/`（把图片交给智能体的窗格） |
| `Packages/MoshpitKit/` | App 和三个扩展都链接的本地 Swift 包：日志、实时活动的数据、加密推送的信封格式和配对存储 |
| `android/` | 安卓 App（Gradle），开发中 |
| `Tests/` | `MoshpitTests/` 单元测试，`MoshpitUITests/` UI 测试 |
| `push-relay/` | 你的主机和 APNs 之间那台不存状态的 Go 转发服务，部署清单在 `deploy/` |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md)、[PUSH.md](docs/PUSH.md)（推送协议从头到尾）、[WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md)、`nsh/`（Rust 核心和 NSH 界面的设计笔记）、其他设计笔记 |
| `scripts/` | `rust/` iOS 静态库的构建和 Swift 绑定，`host/` 装到你开发机上的脚本，`gen/` 生成器，`verify/` 端到端检查，`spikes/` 试验，`wezterm/` 从上游导入 wezterm，`release/` 打包工具 |

App 默认走维护者自己架的推送转发服务；它能看到什么、看不到什么，以及怎么自己架一台，
见 [docs/PUSH.md](docs/PUSH.md)。

## 参与

先读 [CONTRIBUTING.md](CONTRIBUTING.md)，里面有工作流程和代码风格；报告敏感问题前请先看
[SECURITY.md](SECURITY.md)。所有人都遵守[行为准则](CODE_OF_CONDUCT.md)。

## 社区

Moshpit 在 [LINUX DO](https://linux.do) 上分享过。有问题、有想法，在那边说和在 issue 里说都一样欢迎。

## 许可

Moshpit 是自由软件，以 [GNU 通用公共许可证 v3.0](LICENSE)（GPL-3.0-only）发布。你可以按这份许可的条款
构建、修改和再分发它。App Store 上的版本由持有版权的维护者发布。第三方组件保留各自的许可，列在
[NOTICES.md](NOTICES.md) 里。
