# 终端内核：一个 Rust 终端引擎，三个平台各自画（Spike T1）

作者：Cluas（Claude 起草）　评审人：Cluas　状态：已定（wezterm-term，我们的分叉）　更新：2026-11-12
相关链接：设计 [`DESIGN.md`](DESIGN.md)（3.2 终端组件、第 4 节方案 C）· 进度和老 bug 账 [`PROGRESS.md`](PROGRESS.md) · SwiftTerm 分叉补丁 [`../PATCHES.md`](../PATCHES.md) · wezterm 分叉补丁 [`../WEZTERM-PATCHES.md`](../WEZTERM-PATCHES.md)
分支 `nsh-spike/t1-terminal-core`，提交见第 9 节。

给谁看：做 Moshpit 的人和替他干活的 agent。解决什么问题：现在每个平台一个终端模拟器（iOS 和 Mac Catalyst 是 SwiftTerm 分叉，66 个补丁；Android 是 Termux 的 terminal-emulator），同一个 bug 要在两套代码里各修一遍。2026-11-12 Cluas 定了方向：改用一个共享的 Rust 终端内核，候选是 wezterm 的 `wezterm-term` 和 `alacritty_terminal`。第一轮比较之后 Cluas 选了 **wezterm-term**——它的图片（kitty、sixel、iTerm2）和只要 3 MB 的一万行历史，比 alacritty 的速度和体积更要紧。本文记录两轮的证据、我们为 wezterm-term 做的分叉、`crates/term` 现在的样子、迁移计划和过线的门。

范围：内核只管「读字节、维护屏幕状态」：解析、屏幕、历史、选区、搜索、链接、图片、模式。画字、输入法、手势、滚动手感、无障碍仍归每个平台的原生视图。

## 0. 结论和门（先读这个）

**用 wezterm-term，作为我们自己的分叉放在 `vendor/wezterm`，包在 `crates/term`（`moshpit-term`）后面。** alacritty_terminal 退出 App 的构建，只留在比较工具 `spike/term-core` 里当对照（工具后来删了，见第 3 节）。

wezterm-term 原样有四个硬伤，分叉都修了（`docs/WEZTERM-PATCHES.md`，17 个补丁，W0–W17）：

1. **iOS 上编不过**（经 termwiz 依赖的 `termios` 没有 iOS）：termwiz 的终端驱动部分挪进一个 feature（W1）。现在 iOS、Android 都能编。
2. **行尾放不下的宽字符画出界**（W5），**字素被读取切开就变样**（W6：组合字符、ZWJ、VS16、肤色、国旗）：按 xterm 和 tmux 改。
3. **每个终端一个写线程，回复异步到达**（W4）：可关，关了以后回复按请求顺序同步写进我们的输入队列。
4. **2026 同步输出、选区、搜索、OSC 52 查询都不在终端里**：我们在 `crates/term` 里补（2026 按 150 ms 到点放行；选区和搜索建在 wezterm 的稳定行号上）。

修的过程中，`crates/term` 的测试（随机切分、tmux 真值）又在 wezterm 里找出 6 个它自己的 bug（W9 `with_phys_lines` 读错行或越界 panic、W11 宽字符行尾丢折行标记、W12 相邻单元格被读成一个字素、W13 HTS 越界 panic、W14 CUP 停在最后一列之外、W15 新的历史上限不生效），用真工具录的图片又找出 2 个（W16 kitty 自家 `icat` 的无填充 base64 解不开、W17 ECH 把 kitty 图片一起擦掉）。全部修了，每个带测试；上游自己的 148 个测试（`wezterm-term` 53、`wezterm-surface` 40、`wezterm-escape-parser` 55）照样通过，W14 有意改了其中 3 处断言。

**结果**（这台 M2，负载 4–11，数字只用于比较）：

| | SwiftTerm 分叉（现状） | `crates/term`（wezterm 分叉） |
| --- | --- | --- |
| 50 个屏幕用例 | 46 | **50**，另有 6 个分叉修复的用例 |
| 14 个有 tmux 真值的录制流 | 13 | 13（剩下的是 tmux 自己的怪行为） |
| 字节在哪里切开结果都一样 | 是 | 是（全部录制流 6 种切法，随机转义序列 2 万轮） |
| 吞吐（录制流合集） | 7.5 MB/s | 29.6 MB/s（各负载 27–43） |
| 一万行历史（80 列 / 200 列） | 20.6 / 50.3 MB | **3.2 / 5.6 MB** |
| 图片 | sixel、kitty、iTerm2 | sixel、kitty、iTerm2（真工具输出全过） |
| 体积（iOS，剥离后） | — | 内核 +2.0 MB；`crates/term` 全部 C 接口 +3.3 MB（其中正则 App 已有） |

代价，直说：

- **慢。** 比 alacritty 慢 4–8 倍（它 140–250 MB/s），比上游 wezterm 慢 10–40%（我们的字素和单元格检查）。比 SwiftTerm 分叉仍快 2–4 倍，而且解析不再占主线程。一次按键回显：喂一个字 125 ns，取一帧 1.6 µs（p50）。
- **要自己维护一个分叉。** 17 个补丁，上游一年只有 7 个提交进 `term/`，合并负担小，但 bug 要自己修（这次修了 10 个：W5、W6 和上面 8 个）。
- **整屏一滚就要重画。** wezterm 用每行的序号记录改动，我们的帧在视口滚动时给整屏；渲染要上 Metal。

**门 T1**（第 8 节详写）：iOS 上新终端组件做成开关，SwiftTerm 分叉一直是默认和退路，直到 iPad mini（iOS 18.2）和 iPhone 16 Pro 上量出来同时满足：正确性（`cargo test -p moshpit-term` 全绿、录制流和 tmux 真值一致、旧 bug 账每条有测试且通过），手感不输分叉（W11g 的量法：回显 p50/p95 ≤ 7.5/10.7 ms，键盘起落 0/12 跳动、尺寸只变 1 次，切 pane 上屏 ≤ 57 ms，50 MB 突发不乱序不丢、主线程单次卡顿 < 32 ms），解析在真机上 ≥ 10 MB/s，图片三种协议在真机上画对、内存守住上限，输入法、选区手柄、硬件键盘、无障碍真机通过，每 pane ≤ 8 MB（一万行，80 列），Canary 一周无相关崩溃。**不过**：iOS 继续用分叉，`crates/term` 照样拿去替换 Android 上的 Termux，iOS 按没过的那一项单独修。

## 1. 背景

现状（读 `aaed7c0a`）：

- iOS 和 Mac Catalyst：SwiftTerm 分叉 `7e5b936`，66 个补丁。按 `PATCHES.md` 自己的分类，34 个在模拟器层，32 个在 iOS/Catalyst 视图层（输入法、选区、滚动、指针、硬件键盘、局部重画、Metal）。
- Android：Termux v0.118.3 的 terminal-emulator 和 terminal-view 原样入库，改了两处：会话改读写字节通道，吃掉 screen 的 `ESC k` 标题（和分叉补丁 66、B257 是同一个问题）。
- `DESIGN.md` 第 4 节方案 C（一个 Rust 终端引擎，绘制留给平台）当初被否，理由是手机终端最难的是输入法、选区手柄、滚动手感，而分叉在 iOS 上的修复会全部作废。这个判断要拆开看：32 个视图层补丁确实要在新视图里重新解决（问题不变，代码要重写）；34 个模拟器层补丁是换内核就要重新验证的东西——本次逐条对过，能在内核上测的都写成了 `crates/term` 的测试（第 7.4 节）。

结构上变什么（相对 `DESIGN.md` 3.2、3.3）：解析从主线程挪到内核。现在字节通道的读端是原生视图，它在主线程把字节喂给 SwiftTerm；以后读端是 `crates/term` 的 pane 线程，按序读空队列喂给模拟器，视图只拿「帧」——上一帧以来变了的行的拷贝——去画。两条规矩不变：每个方向一个队列，不乱序、不丢。按键照旧经 `mp_stream_write` 同步写进输入队列；模拟器的回复由 pane 线程写进同一个队列。

## 2. 候选

### 2.1 对比表

测量都在这台 Mac mini（Apple M2，macOS 27.0.1）上做，机器共享，负载 4–30，数字只用于互相比较。来源和复现命令见第 3 节、第 9 节。「wezterm 上游」是 `ce8e6b1` 原样（第一轮）；「分叉」是 `vendor/wezterm` 加我们的补丁，按上游默认开关；「crates/term」是分叉加我们这层、开关按 Moshpit 打开。

| 项 | alacritty_terminal 0.26.0 | wezterm 上游 `ce8e6b1` | wezterm 分叉 | `crates/term` | SwiftTerm 分叉 `7e5b936` |
| --- | --- | --- | --- | --- | --- |
| 许可证 | Apache-2.0 | MIT | MIT | GPL-3.0-only + MIT | MIT |
| 取得 | crates.io，约半年一版 | 只能 git 引用；wezterm 上次发版 2024-02 | `vendor/wezterm`，导入脚本可合并上游 | 工作区 crate | 我们的 GitHub 分叉 |
| 维护（近 365 天） | 该目录 10 个提交 | `term/` 7 个，`termwiz/` 8 个 | 我们 + 上游合并 | 我们 | 我们 |
| iOS 能否编 | 能 | **不能**（`termios`） | 能 | 能 | 能 |
| 依赖（包数，`cargo tree`） | 26 | 约 250（Android，默认 feature） | 136（iOS） | 140（iOS） | — |
| 体积，iOS / Android（剥离的 cdylib 增量） | +0.10 / +0.12 MB | 编不过 / +3.82 MB | +2.04 / +2.19 MB | 全部 C 接口 +3.32 / +3.68 MB | — |
| 吞吐，录制流合集 | 186 MB/s | 37 | 31.5 | 29.6 | 7.5 |
| 吞吐，各负载 | 141–247 | 33–52 | 29–48 | 27–43 | 7.5–22 |
| 一万行历史，80 列 / 200 列 | 23.2 / 54.4 MB | 3.2 / 5.6 | 3.2 / 5.6 | 3.2 / 5.6 | 20.6 / 50.3 |
| 50 个屏幕用例 | 43 | 46 | 49（DECCOLM 开关默认关） | **50** | 46 |
| 14 个有 tmux 真值的录制流 | 12 | 12 | 13 | 13 | 13 |
| 读取怎么切结果都一样 | 否（vte 截断 UTF-8） | 否（ZWJ、组合字符丢） | 是 | 是 | 是 |
| 回复 | 同步 | 写线程，异步 | 可选同步 | 同步，按序 | 主线程同步 |
| 损伤 | 每行列边界；滚动报整屏 | 每行序号 | 同左 | 变了的行；视口一动给整屏 | 分叉补丁 24–28 |
| 历史、选区 | 有，选区跟文字走 | 有稳定行号；**无选区** | 同左 | 我们的选区（稳定行号） | 有 |
| 搜索 | 正则 | 无 | 无 | 我们的正则搜索 | 有 |
| 同步输出 2026 | 解析器扣字节 | 终端忽略（mux 管） | 同左 | 我们扣，150 ms 放行 | 补丁 45 |
| 按键、鼠标编码 | 前端做 | 有（含 kitty） | 有 | 待接到 C 接口 | 有 |
| 标题、`ESC k` | `ESC k` 打进屏幕 | 有 | 有 | 有 | 吃掉，不报 |
| OSC 52 | 写和读 | 只写 | 只写 | 写和读 | 写和读 |
| 图片 | 无 | sixel、kitty、iTerm2 | 同左，kitty 只收直传 | 解码后按格子交给视图 | 有 |

### 2.2 不测的候选（各一段）

**libghostty-vt。** Ghostty 的可嵌入内核，2026-11-06 发了 1.0（现在 1.0.1），MIT，C 接口承诺 ABI 稳定。解析、屏幕、带脏标记的渲染状态、选区（含手势事件）、搜索、按键和鼠标编码、kitty 图片都有；不建线程；CI 覆盖 iOS 和 Android。没选它是因为 Cluas 把范围定在两个 Rust 候选；工程上的代价是构建要 Zig 0.17。将来值得再看。来源：<https://mitchellh.com/writing/libghostty-1-0>。

**vt100（crate）。** MIT，0.18.1，读屏用：没有选区、损伤、2026、kitty 键盘。太薄。

**SwiftTerm 的内核编到 Android 上。** 要 Swift 的 Android SDK 加 Foundation，再写 JNI 桥，就是 `DESIGN.md` 的方案 E，当初否掉的理由没变；而且共享内核要把解析挪进 Rust 内核，Swift 版放不进去。不现实。

## 3. 测了什么、怎么测

工具原来在仓库的 `spike/term-core/`（自己的工作区）。11-15 和其他 spike 一起删掉了（E6）：要看或者重跑，回到删掉它的那个提交的上一个提交，`git log --diff-filter=D -1 -- spike/term-core` 能找到它。

- **任务**：tmux `-CC` 的 9 段录制（按 pane 解出 `%output` 和 `%extended-output`，按录制的读取和布局变化喂），herdr 的录制帧，mosh 切 pane 的真实抓包（每段后面有 tmux 自己 `capture-pane` 的真值），合成负载（`seq` 20 万行、密集颜色、中日韩加 emoji、Claude Code 的行内重画和全屏两种画法）。
- **真值**：mosh 的四段 tmux 真值；`tmux_oracle.py` 把同样的字节放进私有 tmux 3.6a（`-L t1-oracle`），记下屏幕、历史、光标。这 8 个 pane 的真值存成了 `crates/term/tests/fixtures/tmux-oracle`，是 `crates/term` 的回归测试。
- **用例**：50 个 vttest 式的屏幕用例（`spike/term-core/src/cases.rs`），账上每个能在屏幕上看出来的模拟器层 bug 一个，加 xterm 的基本行为和 tmux 给的字符宽度；`crates/term/tests` 另有 6 个分叉修复的用例，每个都整段喂一次、逐字节喂一次。
- **随机切分**：随机的转义序列（含截断的 UTF-8、`ESC k`、2026、OSC 8、ZWJ、VS16、CUP 越界），随机切开喂，和整段喂的结果必须一样，也不能 panic（`crates/term/tests/corpus.rs`，提交里跑 200 轮，本地 5 000 轮和换种子 2 万轮过）。
- **图片**：真工具离线录下的输出（`spike/term-core/images.sh`）：kitty 自家的 `kitten icat` 0.50.0（PNG 原样、JPEG 由 icat 解成像素、96×96 的大图），term-image 0.7.2（原始 RGB 加 zlib），`wezterm imgcat`（iTerm2 协议，PNG、JPEG、GIF；它要问终端尺寸，`ptyrun.py` 在一个 pty 上替终端回答），ImageMagick 7.1.2 的 sixel。
- **分叉对照**：`swift/` 用 SwiftPM 编分叉 `7e5b936`，同样的任务和用例出同样格式的快照（`stsnap`）和 bench（`stbench`）。
- **bench**：吞吐是每个负载按 4 KB 一读喂给新终端（一万行历史），5 次取最好；内存是 malloc 所有 zone 的在用字节；体积是 `size/` 的 cdylib 探针（fat LTO、opt 3、剥离），和只有标准库的基线相减。

复现：`cd spike/term-core && cargo run --release --bin corpus && python3 -I tmux_oracle.py && cargo run --release --bin cases && python3 -I compare.py`；`cargo run --release --bin bench`；`size/build-sizes.sh`；`cargo test -p moshpit-term`；`cargo bench -p moshpit-term --bench feed`。

### 3.1 正确性

50 个屏幕用例：alacritty 43、wezterm 上游 46、SwiftTerm 分叉 46、`crates/term` 50。上游 wezterm 错的四个：行尾放不下的宽字符不换行（5 列里写 `abcd中`，`中` 画在第 5 列、出界）、DECCOLM 清屏、ZWJ 跨读取、组合字符跨读取。分叉错的：B184（EL 后折行标记不清，账上还是「攒批」）、`ESC k` 吃掉了但不报标题、窄屏重排丢字（只在本工具里看到，待核实）。

14 个有真值的任务：`crates/term` 13。没过的那个是合成的中日韩任务：`⚠️` 正好落在最后一列时，tmux 把它整个丢了，其他各家都保留。不追。

跨读取一致：上游 wezterm 在 `tmux-cjk-emoji` 第 36 行不同（ZWJ 落在下一次读取开头被删）。分叉加 `crates/term`：全部录制流按录制的切法、整段、每 1/2/3/7/4096 字节切都一样，随机转义序列 2 万轮也一样。一个已知例外写在 W6 里：wezterm 给 `9`+VS16 两列、`9`+VS16+VS16 一列，两个选择符之间被切开又正好在最后一列时，结果会落到下一行；连续两个 VS16 不是程序会发的东西，随机测试排除了这一对。

### 3.2 旧 bug 账逐条

| 账 | 层 | wezterm 上游 | `crates/term` |
| --- | --- | --- | --- |
| B20 每次更新整屏重画 | 视图 | — | 帧只带变了的行；原型回显只重画 1 行 |
| B78 ED 1 清错行的折行标记 | 模拟器 | 过 | 过（`tests/cases.rs`） |
| B79 ED 3 后选区落到别的字上 | 模拟器 | 无选区 | 过：行离开历史，选区就走 |
| B97 重置后终端释放不掉 | 模拟器 | 不适用 | 不适用（Rust 值，`mp_term_close` 停线程） |
| B98 全选复制 65 MB | 模拟器 | 无选区 | 过：代价只是文字本身 |
| B99 备用屏里 ESC c 光标不归位 | 模拟器 | 过 | 过 |
| B100 ESC c 后选区还在 | 模拟器 | 无选区 | 过 |
| B173–B176 指针、连发、Tab、自动填充 | 视图 | — | 新视图待做 |
| B177 有历史时自动换行标错行 | 模拟器 | 过 | 过 |
| B178 关掉的 pane 泄漏 | 视图 | — | 新视图的显示链接要弱持有 |
| B179 同步输出每帧复制整个缓冲区 | 模拟器 | 不做 2026 | 过：扣动作，不复制 |
| B180 历史满后选区漂移 | 模拟器 | 无选区 | 过：稳定行号 |
| B181 切屏不清选区 | 模拟器 | 无选区 | 过 |
| B182、B217 DECCOLM | 模拟器 | 清屏 | 过（W7，什么都不做） |
| B183、B215 选区少最后一列 | 模拟器 | 无选区 | 过 |
| B184 EL 不清折行标记 | 模拟器 | 过 | 过（分叉还没修） |
| B213 回收行带着旧折行标记 | 模拟器 | 过 | 过 |
| B214 备用屏按 25 行建 | 模拟器 | 过 | 过 |
| B216 历史变短不通知选区 | 模拟器 | 新上限根本不生效 | 过（W15 让上限生效，选区跟着走） |
| B218 滚动区下方自动换行、IND | 模拟器 | 过 | 过；输入框预测、⌘ 卡住属视图，待做 |
| B237 取消的按压不发释放（kitty） | 编码 | — | 编码接进 C 接口时带测试 |
| B238 `clear` 后不变行号回跳 | 模拟器 | 过（`StableRowIndex` 6 → 6） | 过；宿主不再持有行号 |
| B239 图片行计数 | 模拟器 | 不适用 | 不适用 |
| B257 / Termux 的 `ESC k` | 模拟器 | 过 | 过，报成标题 |

### 3.3 吞吐（MB/s，4 KB 一读，5 次最好，负载 4–5）

| 负载 | 大小 | alacritty | wezterm 上游（负载 11） | wezterm 分叉 | `crates/term` | SwiftTerm 分叉 |
| --- | --- | --- | --- | --- | --- | --- |
| `seq 1 200000` | 1.5 MB | 174 | 36 | 29 | 27 | 11.7 |
| 密集颜色 | 6.0 MB | 151 | 52 | 46 | 41 | 22.2 |
| 中日韩 + emoji | 2.1 MB | 141 | 48 | 29 | 27 | 9.0 |
| Claude Code 行内帧 | 3.2 MB | 247 | 35 | 32 | 30 | 7.6 |
| 全屏 TUI 帧 | 5.2 MB | 177 | 51 | 48 | 43 | 20.7 |
| 录制流合集 | 4.2 MB | 186 | 37 | 32 | 30 | 7.5 |

分叉比上游慢的部分是 W6 和 W12：每印一个字记下它的格子，每加一个非 ASCII 格子查一次字素边界（常用文字有快速通道，第一版中日韩只有 11 MB/s，加了快速通道到 29）。`crates/term` 再慢 5–12%，是我们的解析循环（先把动作收进一个 Vec，为了 2026 和 OSC 52）。一次按键回显（`cargo bench -p moshpit-term --bench feed`，一千行历史的 80×24）：喂一个字 p50 125 ns、p99 208 ns；取帧 p50 1.6 µs、p99 2.7 µs。

### 3.4 内存（一万行历史，malloc 在用字节之差）

| | 空终端 | 80×24，shell 风格行 | 200×50，满行 |
| --- | --- | --- | --- |
| `crates/term`（wezterm 分叉） | 1.41 MB | 3.16 MB | 5.59 MB |
| alacritty | 2.20 MB | 23.2 MB | 54.4 MB |
| SwiftTerm 分叉 | 0.15 MB | 20.6 MB | 50.3 MB |

wezterm 把滚进历史的行压成「文字串 + 属性段」，所以一万行只要 3 MB 左右，比分叉省 6.5–9 倍。空终端的 1.4 MB 是按历史上限预留的行队列。图片另算，有上限：单张解码后 ≤ 32 MB，kitty 存的图合计 ≤ 64 MB，解码缓存 ≤ 64 MB（`Options`）。

### 3.5 体积（剥离后的 cdylib 增量，fat LTO，opt 3）

| 目标 | 基线 | wezterm 上游 | wezterm 分叉（探针） | `crates/term` 全部 C 接口 | alacritty |
| --- | --- | --- | --- | --- | --- |
| `aarch64-apple-ios` | 283 112 B | 编不过 | +2.04 MB | +3.32 MB | +0.10 MB |
| `aarch64-linux-android` | 273 288 B | +3.82 MB | +2.19 MB | +3.68 MB | +0.12 MB |

分叉里约 0.6 MB 是图片解码（PNG、JPEG、GIF、WebP，W3 去掉了其余格式和 rayon）。`crates/term` 多出来的主要是搜索用的 `regex`；App 经 NSH 已经链了它，进 App 时不会再加一份。G6 的门是安装包增大 ≤ 15 MB。

### 3.6 接口适配

- **字节通道、离开主线程**：`Terminal` 是普通值，放在 pane 线程上喂；W4 关掉写线程以后，回复在解析的同一次调用里按请求顺序写进我们的缓冲，pane 线程再写进输入队列（`tests/frame.rs` 验证回复和按键走同一队列、两万行突发不乱序）。
- **损伤**：wezterm 每行有序号，我们记下上一帧的序号和视口顶行；视口顶行变了（滚动、换屏）给整屏，否则给序号变了的行加光标的新旧两行。
- **历史、选区、搜索**：都建在 wezterm 的稳定行号上：一行在滚动中号码不变，离开历史的行号码小于第一行。选区记下做出时的序号，它下面任何一行变了或离开历史，选区就走（「是它的文字，否则什么都不是」）。
- **图片**：wezterm 把图片挂在它覆盖的格子上（每格一块纹理坐标）。帧按行给出连续格子的图片段，加上解码好的 RGBA（同一份数据同一个 id，视图按 id 缓存纹理）。iTerm2 的图存的是原文件，我们第一次画时解码。
- **按键、鼠标**：wezterm 有完整的编码（`key_down`、`mouse_event`，含 kitty 五个标志），回复走同一个写口。下一步把它们接进 C 接口，三个平台共用，SwiftTerm 分叉里的 iOS 键位表只留「物理键 → 键码」那一层。

## 4. 为什么最后是 wezterm-term

第一轮我推荐 alacritty（快、小、原样能上手机、选区和 2026 现成）。Cluas 选了 wezterm-term，理由是图片和历史内存：Moshpit 用户在手机上看智能体，`imgcat` 和图片预览在开发者里越来越常见，换成 alacritty 就是功能倒退；一万行历史 3 MB 对比 23 MB，开四个 pane 的 iPad 差出 80 MB。

这个选择的代价已经付了大半：编不过 iOS、字素和宽字符的错、写线程、2026、选区、搜索，都在分叉和 `crates/term` 里解决了，第 3 节的数字是解决之后的。剩下的代价是速度（第 0 节）和分叉的维护。

分叉怎么维护（详见 `docs/WEZTERM-PATCHES.md`）：`vendor/wezterm` 保持上游的目录结构；`scripts/wezterm/import.sh` 原样导入上游一个提交，我们的修改是导入之后的提交；升级就是从上次导入的提交拉一个分支、导入新版本、合并回来，冲突只会出在我们改过的地方。凡是嵌入方可能不要的行为都做成 `TerminalConfiguration` 的开关，默认是上游的行为，方便把修复提回上游（W1、W2、W5、W6、W9、W11–W16 都值得提）。

## 5. `crates/term` 现在有什么

`moshpit-term`：我们的 wezterm-term 分叉包在 Moshpit 自己的接口后面，App 其他地方不出现 wezterm 的类型。接口和第一轮一样（iOS 原型和以后的 Android 视图都用它），多了图片：

- `Terminal::feed`（字节可以在任何地方切开）、`take_replies`（按序）、`take_events`（标题、响铃、OSC 52 写和读）、`answer_clipboard`、`sync_deadline` / `flush_sync`、`discard_pending_input`、`resize`、`set_scrollback`、`set_palette`。
- `frame()`：上一帧以来变了的行的拷贝，每格颜色算好（调色板、反显、暗淡、隐藏、粗体变亮），标出宽字符、占位、选中、搜索命中、链接；每行的图片段（`ImageRun`：格子、纹理矩形、z）和这一帧用到的解码图片（`DecodedImage`：id、宽高、RGBA）；光标。
- `scroll`、`scroll_to_bottom`、`display_offset`、`history_len`；`select_start`（字符、词、行、块）、`select_extend`、`select_clear`、`selection_text`、`selection_bounds`；`search`、`search_clear`；`link_at`（跨折行）；`modes()`；`visible_text()`、`row_text()`。
- 我们这层做的：2026 同步输出（扣住动作，150 ms 或 200 万个动作放行，软复位结束）；OSC 52 查询交给宿主；全复位和切屏结束选区；选区、搜索、视口；图片解码缓存（按哈希，按字节预算淘汰）。
- `pane`：一个终端接一条字节通道，自己一个线程；`ffi` 和 `include/moshpit_term.h` 是给原生视图的 C 接口（新增 `MpImageRun`、`MpImage`、`mp_frame_image`）。

测试（59 个，`cargo test -p moshpit-term`）：屏幕用例 30 个（旧 bug 账和分叉修复，每个整段一次、逐字节一次）；录制流 5 个（mosh 对 tmux 真值、8 个 tmux pane 对真 tmux 3.6a、全部录制流 6 种切法、herdr 帧、随机转义序列）；帧和字节通道 6 个；选区和搜索 9 个；图片 9 个（四种工具的输出、kitty 分块传输、按字节切开、同一张图只解码一次、三种上限、kitty 的 t=f 和 t=t 被拒且文件原封不动）。

## 6. 原型显示了什么

`spike/ios/Spike/CoreFrameRenderer.swift`：Spike App 里的原生组件 `CoreTermView`。模拟器在 pane 线程跑；唤醒一次只跳一次主线程；主线程拿帧、只让变了的行失效；`draw(_:)` 用 CoreGraphics 和 CoreText 画背景、文字、下划线、光标，现在也画图片：每行的图片段按 z 画在文字下面或上面，解码好的图片按 id 做一次 `CGImage`。它对 iPhoneOS SDK 类型检查通过，接进了 Spike App；按要求没有在模拟器或设备上跑。

同一份 Swift 文件在这台 Mac 上画进位图（`spike/term-core/render/run.sh`，无窗口，3 倍，Menlo 13 pt；图片那一帧的「拿整帧」含四张图的复制）：

| 静帧 | 拿整帧 | 画整帧（30 次中位） | 回显：拿帧 / 画 1 行（300 次中位） |
| --- | --- | --- | --- |
| mosh 切 pane 真实抓包 90×50 | 0.17 ms | 3.3 ms | 0.003 ms / 0.027 ms |
| 真工具的图片 80×24（kitty 96×96、iTerm2 PNG 和 GIF、sixel） | 0.28 ms | 0.65 ms | 0.002 ms / 0.024 ms |
| Claude Code 行内帧 100×40 | 0.10 ms | 1.6 ms | 0.002 ms / 0.029 ms |
| 全屏 TUI 120×50 | 0.12 ms | 3.9 ms | 0.001 ms / 0.015 ms |
| 密集颜色 120×40 | 0.14 ms | 4.7 ms | 0.002 ms / 0.029 ms |

PNG 在 `spike/term-core/out/render/`（不入库）：中日韩在格子上、`⚠️` 两列、框线连续、反显和背景色擦除都对，四张图片在各自工具把它们放的位置。能说明的：拿帧几乎不花时间，回显只重画一行，图片能按格子交给视图画。不能说明的：真机上的合成、输入法、手势、滚动——这些只有门 T1 在 iPad 上量。整屏重画 1.6–4.7 ms（CPU，负载 5.7）说明正式版要上 Metal。

## 7. 迁移计划

### 7.1 结构

- 内核侧：会话照旧往 pane 的 `Stream` 推字节；`crates/term` 的 `Pane` 在内核里读它。tmux、mosh、herdr 都不用改。
- 视图侧：iOS 的终端组件从「读字节喂 SwiftTerm」变成「拿帧、画行和图片」；按键照旧 `mp_stream_write`，按键编码改由内核做（wezterm 的编码器）。
- 链接识别照旧放内核，直接读 `visible_text()`。
- mosh 预测回显：由内核在帧上叠一层（预测的格子带标记，确认后去掉），三个平台共用；在那之前 mosh 会话的新视图开关不打开。

### 7.2 iOS 先

1. **内核补齐（1–2 周）**：把 wezterm 的按键和鼠标编码接进 C 接口（物理键 → wezterm 键码，kitty 取消按压的释放 B237 带测试）；粘贴（括号粘贴，用 `moshpit-herdr` 已有的清理）；预测回显叠加层；把 W1、W5、W6、W9、W11–W16 提给上游。
2. **`CoreTermView` 正式版（8–11 周）**：
   - 渲染（2–3 周）：Metal、字形图集、预热（G1：首帧新字形 30–37 ms）、只重画变了的行、滚动用位移加补新行；图片作纹理。
   - 输入法和文字输入（1–2 周）：分叉补丁 5–7、9–12、19、23、47、60 的行为。
   - 选区手柄和菜单（1 周）：补丁 4、13、15–17、20、55、56。
   - 滚动手感（1 周）：补丁 21 和 `project_scroll_architecture` 的规则。
   - 硬件键盘和指针（1 周）：B174、B175，补丁 48–52、61。
   - 无障碍（3–5 天）：每行一个无障碍元素，VoiceOver 读可见行和光标行。
   - Mac Catalyst（3 天）：G4 的各项复测。
   - 真机验收（1–2 周）：第 8 节的门。
3. **开关**：隐藏的核心开关页加 `MOSHPIT_CORE_TERMINAL`，按 `DESIGN.md` 3.6 的流程：Canary 包默认开、商店包默认关；门过了，下一个商店版默认开；分叉在商店里再留两个版本才删。

### 7.3 Android 后（2–3 周）

Termux 的 terminal-view 留着（它的 `InputConnection`、手势、选区手柄在 G3 和 W12 里验过），只换掉模拟器：写一个适配层让它的渲染器从我们的帧读格子，图片画成 Bitmap，经 JNI 调 `mp_term_*`。重跑 W12 的模拟器测试和 `android/live.sh`。Termux 的两处修改随之作废。

### 7.4 66 个分叉补丁的去向

| 类 | 补丁 | 去向 |
| --- | --- | --- |
| 模拟器层，wezterm 或我们这层做对了，有测试 | 1、2、18、22、25、29、32、35–42、44、53、54、55（模拟器半）、57、59、63（稳定行号，B238） | 不用再带；测试在 `crates/term/tests` |
| 模拟器层，分叉或我们这层修的 | 58、64（W7）、66（wezterm 本来就处理 `ESC k`）、19（`discard_pending_input`）、43（W7） | 有测试 |
| 模拟器层，不再适用 | 33、34、45、65 | 历史按需长、Rust 值无引用环、2026 不复制缓冲区、图片挂在格子上随行一起裁掉 |
| 宿主接口 | 3、8、14 | 换成 `visible_text()`、`link_at`、`Event::ClipboardLoad` |
| iOS / Catalyst 视图层 | 4–7、9–13、15–17、20、21、23、24、26–28、30、31、46–52、56、60–62 | 第 7.2 节第 2 步逐条在新视图里重做，每条一个测试或真机检查项 |

### 7.5 成本合计

| 部分 | 估计 |
| --- | --- |
| 内核补齐（接上编码、粘贴、预测叠加层、上游提交） | 1–2 周 |
| iOS 新视图到过门 | 8–11 周 |
| Android 换模拟器 | 2–3 周 |
| 分叉维护 | 每次合并上游约半天（上游 `term/` 一年 7 个提交） |
| 合计 | 11–16 周（一个人加 agent），iOS 和 Android 可以部分并行 |

### 7.6 风险

- **视图手感**最大：分叉的 32 个视图补丁是真机上一个个踩出来的，新视图会重新踩一遍。所以分叉一直是默认和退路，门在真机上量。
- **吞吐**：M2 上 27–43 MB/s，iPad mini（A15）估计再低三成。解析在 pane 线程上，不卡主线程，但 `cat` 大文件、全屏 TUI 刷屏时这条线程更忙、更费电。门里要求真机 ≥ 10 MB/s；不够就先优化 W6/W12 的检查（例如只在非 ASCII 的读取上做），再看上游的打印路径。
- **整屏重画**：视口一动就是整屏，渲染要快（Metal），否则刷屏时卡主线程。
- **分叉**：17 个补丁要跟着上游合并；上游不活跃时 bug 要自己修。

## 8. 门 T1

门不过就按右列改路。量法沿用 G1、W11g：iPad mini（iOS 18.2）和 iPhone 16 Pro（iOS 27.3），同一个 NSH 终端屏里新视图和分叉各跑一遍。

| 门 | 验证什么 | 过线标准 | 不过怎么办 |
| --- | --- | --- | --- |
| T1a 内核正确性 | `crates/term` 和 `vendor/wezterm`，在 Mac 和 CI 上 | `cargo test -p moshpit-term` 全绿；`vendor/wezterm` 上游测试全绿；所有录制流和 tmux 真值一致；任意切法结果一致；新录的流（Claude Code 真跑一段、vim、htop、less）加进真值后同样一致 | 在分叉或 `crates/term` 里修，记进 `WEZTERM-PATCHES.md` |
| T1b 旧 bug 账 | 第 3.2 节和第 7.4 节每一条 | 模拟器层每条有 Rust 测试且通过；视图层每条有 XCTest 或真机检查项且通过 | 不过的那条阻塞 iOS 开关默认打开；Android 不受影响 |
| T1c 手感 | iPad、iPhone，W11g 的量法 | 回显 p50/p95 ≤ 7.5/10.7 ms；键盘起落 12 次里 0 次可见跳动、尺寸只变 1 次；切 pane 上屏 ≤ 57 ms；50 MB 突发不乱序不丢（哈希一致），主线程单次卡顿 < 32 ms；按键到回显那一帧主线程 p95 ≤ 1 ms | 量出慢在哪一跳：拿帧、画、合成；渲染不够就上 Metal 和滚动位移 |
| T1d 解析 | 真机，pane 线程 | 录制流合集 ≥ 10 MB/s；突发时 pane 线程以外的 CPU 不高于分叉 | 优化 W6/W12 的检查和我们的动作缓冲；还不够就看上游打印路径 |
| T1e 图片 | 真机，真工具 | kitty（icat）、sixel、iTerm2（imgcat）在各自位置画对；单张 > 32 MB、kitty 合计 > 64 MB、解码缓存 > 64 MB 都守住；kitty 的 t=f/t=t/t=s 被拒 | 按协议修；不行的协议先关掉，别的照常 |
| T1f 输入法、选区、键盘、无障碍 | 真机测试加人手三项 | 系统拼音、日文假名组字；按住删除连发 ≥ 5 次且按序；长按选词、手柄拖到最后一列、编辑菜单；硬件键盘连发、kitty 释放、⌘ 不卡；VoiceOver 读出可见行。人手：第三方输入法（豆包）、滚动手感、点和长按互斥 | 按项修；手感三项不过，iOS 不开默认 |
| T1g 资源 | 真机 | 每 pane ≤ 8 MB（一万行，80 列，不含图片；现在 3.2 MB）；安装包增大 ≤ 4 MB（现在 +3.3 MB，其中正则 App 已有） | 看是哪个依赖撑大了包（图片格式、正则） |
| T1h 上线 | TestFlight Canary | 一周内没有终端相关崩溃、没有相关反馈 | 发默认切回分叉的版本，修完再进 Canary |

## 9. 来源

本仓库（分支 `nsh-spike/t1-terminal-core`）：

- 第一轮：`19465931` 对比工具；`d9e43002` `crates/term`（alacritty 版）；`f7605060`、`32172eca`、`73d08464`、`e0141c42`。
- 第二轮（wezterm）：`61933440` 原样导入上游 `ce8e6b1`；`8685d981` W1–W3（iOS 能编、kitty 只收直传、四种图片格式）；`c86c6877` W4–W15；`8cb773e8` W16–W17；`0232b65b` `crates/term` 换成 wezterm 分叉、加图片；`dd809a57` 原型画图片、工具改测分叉；`3af393e5` 回显的量法。
- 录制流：`crates/tmux/tests/fixtures/captures`、`crates/herdr/tests/fixtures/recorded/frames.jsonl`、`Tests/MoshpitTests/Fixtures/mosh-switch`；tmux 真值 `crates/term/tests/fixtures/tmux-oracle`；图片 `crates/term/tests/fixtures/images`（生成：`spike/term-core/images.sh`）。
- 补丁表：`docs/PATCHES.md`（SwiftTerm）、`docs/WEZTERM-PATCHES.md`（wezterm）；Termux 改动 `android/termux/NOTICE`。

外部：

- wezterm 主干 `ce8e6b1`：<https://github.com/wezterm/wezterm>（`term/src/terminalstate/performer.rs` 的 `flush_print`、`mod.rs` 的 `ThreadedWriter`、`mux/src/lib.rs` 的 2026、`wezterm-surface/src/line/clusterline.rs`）。
- alacritty `alacritty_terminal_v0.26.0`：<https://github.com/alacritty/alacritty>；vte 0.15.0 和主干 `89c9c22` 的 `advance_partial_utf8`：<https://github.com/alacritty/vte>。
- libghostty-vt 1.0：<https://mitchellh.com/writing/libghostty-1-0>。
- 图片工具：kitty 的 `kitten` 0.50.0（<https://github.com/kovidgoyal/kitty/releases/tag/v0.50.0>）、term-image 0.7.2（PyPI）、wezterm 20260929-043349-cab25161 的 `imgcat`、ImageMagick 7.1.2-27。
- SwiftTerm 分叉 <https://github.com/Cluas/SwiftTerm> `7e5b936`；tmux 3.6a（Homebrew），跑在私有的 `-L t1-oracle`。
