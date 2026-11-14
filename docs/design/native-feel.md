# Moshpit 原生质感改造记录

日期：2026-09-11。范围：Moshpit iOS App（SwiftUI，iOS 18+，Xcode 26）。P0–P3 已全部落地（分支 v2）。
最初的方案（改造前的 grep 计数、逐屏现状、A–H 分项建议、首页前后对照图和分期估时）描述的是改造之前的
App，已从本文删去，git 历史里有原文；这里留下结论、原则、每一期实际做了什么，以及不做的事。

## 一句话结论

> **2026-09-11 校准（用户）**：「原生质感」指的是质感与原生 app 相当，**不是让 App 用原生组件**。下文"零件交还系统"的推理是这份方案最初的路线，P0/P1 按它做完后 App 开始长得像
> 设置.app；P3 把 App 自己的视觉语言收回到这些容器上（见文末「P3 回收」）。留着原文，是因为它解释了为什么容器还是系统件：行为（Dynamic Type、滑动操作、编辑模式、
> 键盘避让、玻璃）来自容器，性格来自容器里画什么。

改造前 App 不像原生，根源不是颜色，而是**零件不是系统的**：当时 `Moshpit/UI` 下 27 个视图文件里没有一个 `List` 或 `Form`；
导航页头、分组列表、表单、底部弹层、拖拽编辑手柄、空状态，全是 `VStack`/`HStack` 手画的；字号全部是固定 pt，
不跟随系统字号；没有 Reduce Motion 分支。这些手画件在 iOS 26 上尤其吃亏：系统零件自动拿到 Liquid Glass 的材质、
圆角、转场和玻璃工具栏，手画件永远停在上一代。

方案的核心动作只有一条：**容器和控件交还系统，个性收缩到三处**：终端外壳（顶栏 + 快捷栏）、强调色、图标与 mark。
这样 App 会"免费"和系统同一代，而且往后每个 iOS 大版本都跟着走。

## 三条原则

1. **容器交给系统，内容保留个性。** 导航栏、列表、表单、弹层、编辑模式、空状态、搜索，一律用系统件；行里面画什么随意。
2. **字号用语义，尺寸用系统节律。** `.headline/.subheadline/.footnote/.caption`；边距 16/20，行高 ≥44，8pt 栅格；等宽只给路径、命令、技术值。
3. **动效来自系统。** 导航、sheet、列表增删用默认转场；自定义动画只留给"状态变化的那一瞬"（连上了、需要你了），并且尊重 Reduce Motion。

## 落地记录

### P0 状态（2026-09-11，已完成，分支 v2）

- 首页：`NavigationStack` 大标题 + 右上 `ToolbarItemGroup`（齿轮 / ＋）；`List(.insetGrouped)` 承载状态行、连接卡、页脚；空状态 `ContentUnavailableView`；
  卡片 `swipeActions`（Edit / Disconnect / Delete，Delete 显式 `.tint(.red)`，否则全局 accent 会盖掉 destructive 的红）+ 保留长按 `contextMenu` 与 ⋯ `Menu`。
- 表单：14 个屏幕（设置、添加连接、主题/强调色画廊与编辑器、导入主题、快捷键与新增、SSH 密钥与新增、密钥选择、新任务、
  语音识别/语言/模型）全部 `Form` + `Section(header:footer:)`；`FormGroup` 退化为 `Section` 薄包装，`moshpitForm()` 只做背景替换。
  `LabeledContent` 用在带值的行（端口、UDP 范围、Accent/App Icon/Theme），AX 字号下自动上下堆叠；`ChevronRow` 同样在 AX 字号下堆叠。
- 字体：`Face.display/text/mono` 按最近的系统 `TextStyle` 映射（<9.75pt 保持固定，用于卡片上的 SAVED/SSH 小标）；Dynamic Type 生效。
- 背景：`MoshpitBackground` 已删除，`SignalGrid` 只留在连接中屏；`.background(Ink.screenBG)` 统一。
- Reduce Motion：`TransportPill`、光标预览、漫游横幅、`BreathingIconMark`、`PaneLoaderGlyph`、`PulseDots`、语音 `PulsingDot` 全部分支。
- 验证：iPhone 17 Pro / iOS 26.2 模拟器逐屏截图（默认 + AccessibilityL），`MainFlowUITest`、`ThemeFlowUITest`、`AppearanceFlowUITest` 全绿；
  三个套件里对 `Form` 之下折叠区域的断言改经 `XCUIApplication.reveal(_:)` 滚动到位（`Form`/`List` 惰性建行，视口外的行不在 AX 树里）。
- 已知遗留：iOS 26 不再自动把分组标题转大写，标题文案一律手写大写；首页「Connections」等 `List` 标题保留原样式；
  单元测试里 Push 相关三个套件在 `CODE_SIGNING_ALLOWED=NO` 下因无 App Group 授权而失败，签名构建全绿。

### P0 补丁：Dynamic Type 复查（2026-09-11，分支 v2）

第一轮 AccessibilityL 截图暴露的样式问题及处理（新旧对照见 /tmp/p0 画廊的「对比」一节，旧版取 `2c6d268` 构建）：

- 旧版在 AX 字号下几乎不变——`Face` 全是固定 pt，只有系统导航栏标题会放大；这是 P0 之前对 Dynamic Type 的真实状态。
- 连接卡：卡头在 AX 字号下改为纵向堆叠（图标 → 名称/SSH 小标 → SAVED → 主机 → 电源），名称允许两行；状态小标改 `caption2` 等宽。
- 带数字输入的行（Port / Proxy Port / UDP From / To）统一为 `NumberFieldRow`：默认字号右对齐 100pt 宽，AX 字号下堆叠并左对齐。
- 光标 Color 行、`ChevronRow`、Accent 画廊行（名称 / 十六进制）在 AX 字号下纵向堆叠；快捷键 chip 文本 `lineLimit(1).fixedSize()` + 最小宽度，不再断成 "es/c"。
- 添加快捷键的 quick keys 网格改自适应列宽（AX 字号 104pt 起），"PREFIX" 不再截成 "PRE…"。
- 终端外壳（面包屑、快捷栏、连接中屏）是键盘式 chrome，整体钉在 `.dynamicTypeSize(...DynamicTypeSize.xLarge)`；快捷键页的 PREVIEW 条预览的就是这条栏，同样钉住。
- 连接中屏的标语两行居中。
- 系统行为，不改：`Picker(.segmented)` 的段文字不随 Dynamic Type 放大；`Section` 标题随字号放大。

### P1 状态（2026-09-11，已完成，分支 v2）

- tmux 弹层（Sessions / Windows / Select Pane）：`CompactSheet` 改为 `NavigationStack` + `List(.insetGrouped)`，标题进导航栏，＋ 是 `confirmationAction` 工具栏按钮，
  按键提示进 `Section` 页脚；`presentationDetents([.medium, .large])` + 拖拽指示条取代原来"按内容撑高"的 sheet。iOS 26 上不设 `presentationBackground`
  （任何显式背景都会去掉系统的 Liquid Glass sheet 材质），iOS 18 回退 `Ink.sheet`。`SheetListRow` 退化为普通行：图标 / 名称 / 等宽 meta / 状态点 / 勾选，
  合并为一个 AX 元素并带 `.isSelected`。
- 快捷栏编辑：`Form` 常驻 `editMode = .active`，IN TOOLBAR 用 `.onMove/.onDelete`（系统 ≡ 手柄与 − 圆点），CUSTOM 用 `.onDelete`，AVAILABLE 行前置绿色
  `plus.circle.fill`；手绘的手柄、红圈、`draggable/dropDestination` 全部删除。语义说明：IN TOOLBAR 的 − 只是"下架"（`removeFromToolbar`），CUSTOM 的 − 才是删除。
- 终端顶栏：高度 44pt（紧凑 40pt），返回键与面包屑用 `glassChrome(in:)`——iOS 26 `glassEffect(.regular.interactive())`，iOS 18 `.background(.bar)` + 细边；
  返回键 44×44 热区并带 `accessibilityLabel("Back")`。
- 触感：`Haptics.tap()/select()` 的命令式调用从主题 / App 图标 / 强调色画廊移除，改 `.sensoryFeedback(.selection, trigger:)`；
  终端连接状态用 `.sensoryFeedback(trigger: connState)`：connecting/reconnecting → live 给 `.success`，live → reconnecting/offline 给 `.warning`。
- zoom 转场：**放弃**。`.navigationTransition(.zoom)` 会给被推入的页面装一套系统的"下拉即退出"手势，在终端里往下一划（本该是滚动）整个终端就弹回首页
  （iOS 26.2 模拟器复现；对照组：普通 push 不受影响）。SwiftUI 没有否决该手势的 API，终端保持标准 push，理由写在 `HomeView` 的 `navigationDestination` 旁。
- 验证：本机 sshd + 隔离 tmux 服务器（`-L moshp1`，两窗口三窗格）真连；三个弹层、顶栏、快捷栏编辑（− 下架、＋ 上架、≡ 手柄）逐屏截图；
  `MainFlowUITest` / `ThemeFlowUITest` / `AppearanceFlowUITest` 在 iPhone 17 Pro / iOS 26.2 上全绿。`TerminalScreen` 只改了 `topBar` 子视图与修饰符，identity 未变。
- 测试基建顺手修的两件事：`-MOSHPIT_RESET` 现在也把快捷键存储恢复出厂（`ShortcutStore.restoreDefaults()`），此前每跑一次 `MainFlowUITest`
  就多一个 "tmux prefix"；新建的自定义快捷键会同时出现在 IN TOOLBAR 与 CUSTOM 两组，`staticTexts["tmux prefix"]` 天然二义，测试改用 `firstMatch`。
  另外本机有三台同名 `iPhone 17 Pro` 模拟器（26.2 / 26.4 / 26.5），`-destination name=` 会落到别台，测试与截图一律按 UDID 指定。

### P2 状态（2026-09-11，已完成，分支 v2）

先记一句用户在 P2 进行中给的校准：**「原生质感」指的是质感与原生 app 相当，不是必须用系统组件。** 组件是手段；自绘控件手感到位就保留。P2 因此只做布局、键盘、指针这些"手感"项，不再把东西往系统组件上换。

- iPad 分栏：`HomeView` 在 **iPad 且宽度 regular** 时用 `NavigationSplitView`（左栏 380pt 的连接列表，右栏终端或「No Terminal Open」占位），其他情况仍是原来的 `NavigationStack`。
  iPhone Max 横屏也是 regular，但故意不分栏：横屏手机要的是更宽的终端，而且每次旋转都换层级会把终端重新挂载一遍。`path` 仍是唯一的"哪个终端打开着"的真源，两种布局共用，
  多任务尺寸变化时打开的终端不丢（会重新挂载，`hub.prepare/start` 幂等、SwiftTerm 视图归 session 持有，不重连）。竖屏打开终端时侧栏自动收起，顶栏最左的按钮从「返回」变成「切换侧栏」（`terminal-sidebar`）；⌘W/出错返回时侧栏一并放回来，不留一个没路可走的占位页。
- tmux 三个弹层在 iPad 上改为锚在面包屑胶囊上的 popover（400×380，`AdaptivePresentation`），iPhone 仍是 sheet；判定用 idiom + size class，Max 横屏不受影响。
- 硬件键盘：首页 ⌘N 新连接、⌘, 设置；终端 ⌘K 打开 Windows、⌘1–9 按 Windows 列表顺序切窗口、**⌘W 回首页**（不是文档原先写的"断开"——和返回键同义，会话保持，不会误删；断开仍走卡片菜单）。
  首页两条是工具栏按钮上的 `.keyboardShortcut`；终端的三组放在 scene 的 `.commands`（`TerminalCommands`，菜单「Terminal」）里，动作经 `HardwareKeys` 注册表转给当前 `TerminalScreen`
  （onAppear/onDisappear 注册/注销，owner 令牌防 iPad 切主机时新旧实例互清）。走过两条死路才到这：屏内隐藏的 `.keyboardShortcut` 按钮在键盘收起（无 first responder）时不触发，
  UIKit 从最深 view controller 往上找，走不到 hosting view；把 `PushAppDelegate` 改成 `UIResponder` vend `UIKeyCommand` 则把首页工具栏的 SwiftUI 快捷键一并弄死。菜单命令由菜单系统派发，有没有焦点都行。
  标题会出现在 iPad 长按 ⌘ 的面板里。SwiftTerm 对它不用的 ⌘ 组合会调 `super.pressesBegan` 交给响应链，所以终端持有焦点时也能触发。
- 指针：连接卡卡头、tmux 弹层行、终端返回/侧栏键加 `.hoverEffect(.highlight)`。
- 浅色模式评估：**本轮不做**，但成本已经很低——P0 之后底色与前景全走语义色（`systemGroupedBackground`/`secondarySystemGroupedBackground`/`.primary`…），
  真正卡住的是三处：`Ink` 里仍有 24 个固定 hex（终端色、状态色、chip 底色，其中 chip/`sheet`/`navGlass`/`hairline` 需要成对的浅色值）、所有屏幕显式 `.preferredColorScheme(.dark)`
  （去掉即跟随系统）、终端主题与 App 主题解耦（浅色 App 配深色终端是常态，主题目录不用动）。估 1–2 天，先决条件是用户想要它。
- 验证：iPad Pro 13"（iOS 26.2）竖屏截图——分栏首页、占位、终端全宽、侧栏切换、面包屑 popover；iPhone 上 `KeyboardShortcutUITest`
  用 `typeKey` 走真实硬件键盘事件（⌘,/⌘N 直接跑；⌘K/⌘1/⌘W 需要 `TEST_RUNNER_MOSHPIT_SEED_*` 环境变量指向本机 sshd + tmux，没有就 skip；⌘K 在键盘收起时发、⌘1 在终端聚焦后发，两种响应链都盖住），
  加原有三套 UI 测试全绿。横屏没有自动化手段（模拟器旋转要抢 Mac 前台），靠 regular×regular 同一套代码路径推断。
- 测试基建两个坑（记在 `reference_ios_tools` 记忆里，也记这儿）：① 模拟器只在开机后的第一次 `typeKey` 跑法里送达组合键，第二次起同一构建的 ⌘, 也无反应，
  换设备（iPhone→iPad）立刻又好，重启设备即恢复——跑 `KeyboardShortcutUITest` 前先 `simctl shutdown/boot`；② iPad 模拟器上终端一开，XCUITest 的 idle 等待每步顶满 60s
  （终端测试 11 分钟），iPhone 上没有；探针确认不是分栏状态抖动（整个流程 `columnVisibility` 只变一次，CPU 4–8%），没继续追。
- 已知：竖屏刚打开终端的 2 秒内点侧栏切换键偶尔被吞（分栏收起动画期间的绑定回写），稳定后 100% 可用。

### P3 回收（2026-09-11，分支 v2）

用户看过 P2 后的指示：「写到文档不够，开始调整」——P0/P1 换成系统件之后 App 丢掉的性格，要收回来，不是记一笔了事。做法是**容器不动、皮肤换回自己的**：

- 分组标题回到终端口吻：`SectionKicker`（`Face.mono(10.5, .semibold)`、字距 1.2、`Ink.sectionTitle`、强制大写），`FormGroup` 的 header 统一用它；
  首页 CONNECTIONS 旁边的计数 chip 也回来了。字号挂在 `caption2` 上，仍随阅读字号走。
- 分组底色从系统灰换回墨蓝 `Ink.group`，行间线 `Ink.hairline`（`moshpitRows()`，`FormGroup`/tmux 弹层/主题导入页全部走它）；
  页脚回到 12pt `Ink.tertiary` 的低声说明（`moshpitFooter()`）。旧版分组的 1px 描边没有回来——`Section` 拿不到整组的几何，逐行画会变成行间横线；底色 + 细分隔线已经够辨认。
- 系统分段控件通过 `UISegmentedControl.appearance()` 换成 App 的轨道/浮块色（`Chrome.installAppearance()`，App init 调一次）；Password / SSH Key、Block / Bar / Underline 不再是灰色设置.app 控件。
- 首页：`SignalGrid` 舞台回到首页（和连接中页共用一块地板；表单仍是平的）；品牌卡上的 THE PIT NEVER CLOSES 以 kicker 身份挂在状态行上方；
  状态行回到 mono 语气，底色跟着状态走（有 agent 等你=琥珀洗、在线=强调色洗、安静=墨）；空状态的 server.rack 换成 App 自己的 mark；版本页脚用 `Ink.meta`。
- 快捷键编辑器的分隔线拉到整组宽（chip 宽度不一，系统缩进的分隔线每行起点都不一样）。
- 没动的：大标题导航栏、`List`/`Form`、滑动操作、编辑模式、detents、iOS 26 玻璃、触感、iPad 分栏、快捷键——这些是"手感"，是这轮要保住的。
- 验证：`/tmp/p0/cap.sh` 同一套屏默认 + AccessibilityL 两遍截图，与旧版/P2 三栏对照（`/tmp/p0/compare/triples`）；本机 tmux 真连看 Windows 弹层；三套回归 UI 测试。

风险：`TerminalScreen` 外壳改动必须保证视图 identity 不变（纯视觉变化不能触发 SSH 重连或终端重建）；`Ink.accent`
主题 API 和 `AppIconMark`/`MoshpitMark` 规则保留；72 处 `accessibilityIdentifier` 一个不能丢。

## 不做的事

- 不改终端输出颜色语义、不换正文字体、不加纹理。
- 不把官网的酸绿搬进 App（用户已否决过一次）。
- 不为了"原生"删掉任何功能入口；能力只换壳。
- 不先做浅色模式；语义色到位之后再评估。
