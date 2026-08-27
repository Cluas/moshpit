# 终端触控操作矩阵

一次点按在终端里可能意味着七种不同的事，而 2026-08-27 的三连修（focusOnTap、
按落点分键盘、选择手柄）证明了逐个补洞不可持续：每修一个手势都可能碰倒另一个。
这份矩阵是全量清单——**每一种手势 × 每一种上下文 × 期望行为 × 它由哪一层测试钉住**。
改任何手势代码之前先对照它；改完把新行为写回来。

实现分两层，冲突面也在两层之间：

- **fork（SwiftTerm iOS）**：singleTap（链接/菜单/聚焦门 `focusOnTap`）、
  double/tripleTap（选词/选行）、长按、选择 pan（`panSelectionHandler`）、
  UIEditMenuInteraction 菜单。
- **app（TerminalScrollGesture + ShortcutBar）**：单指 pan（纵=滚动/横=切
  pane）、tap-to-position（`handleTap`，含键盘判定 `tapWantsKeyboard`）、捏合
  缩放、D-pad / scroll thumb / 键盘 toggle。
  让步规则集中在 `TerminalScrollGesture.gestureRecognizerShouldBegin`。

## 上下文定义

| | 上下文 | 判定依据 |
|---|---|---|
| A | 输入区（未聚焦） | 光标行 ±2（`tapWantsKeyboard`），未翻本地历史 |
| B | 输出/历史区（未聚焦） | 其余屏幕区域 |
| C | 已聚焦（键盘上） | `isFirstResponder` |
| D | 翻本地历史中 | `canScroll && scrollPosition < 0.999`（先 canScroll——nothing-to-scroll 时 scrollPosition 也是 0） |
| E | 鼠标模式全屏应用 | Claude Code / vim：alternate screen + DECSET 1000+（`localAppWantsMouse`） |
| F | 选择激活中 | `selection.active`（app 侧经 `canPerformAction(copy:)` 读取） |

## 矩阵

验证层：**U** = 单元测试（纯函数） · **T** = 拓扑测试（recognizer 装配/让步） ·
**S** = 模拟器脚本（idb + 决策日志 `Log.input` / 截图） · **M** = 真机手测。
「层」列 = 已有覆盖；括号 = 待补。

| 手势 | 上下文 | 期望行为 | 层 |
|---|---|---|---|
| 单击 | A | 弹键盘 + click 定位到点按格（E 内） | U+S ✅ |
| 单击 | B | 不弹键盘；仅 click（E 内生效，如 jump-to-bottom；纯 shell 无鼠标模式则无副作用） | U+S ✅ |
| 单击 | C | 键盘保持；click 定位；近光标 4 列×2 行且无鼠标模式 → 粘贴菜单 | U ✅（菜单 M） |
| 单击 | D | 永不弹键盘（光标在屏外）；click 照发 | U ✅ |
| 单击链接 | A/B/C/D | 打开链接，**不**弹键盘、不聚焦（fork patch 4+13+15）。跨行 URL 点任一行都是完整地址：软换行由 `isRowWrapped` 拼接；**硬换行**（Claude Code 自排版，贴右边缘断行+缩进续行）由 PlainLinkDetector 的续接启发式拼接，防误伤门槛=续行段 ≥2 字符且含非字母 | U ✅ / M ✅ |
| 单击 | F | **清除选择**（app `handleTap` → fork `closeSelection()`，聚焦与否都成立；不发 click、不弹键盘） | U ✅ |
| 双击 | 任意 | 选中词/表达式 + 出现手柄 + **Copy 菜单（无需聚焦，UIEditMenuInteraction）** | M（菜单出现 S 待补） |
| 三击 | 任意 | 选整行 + 菜单 | M |
| 长按 | 任意 | **选中指下的词** + 手柄；按住滑动=扩选；松手出菜单（含 Paste）。不弹键盘（fork patch 17） | S ✅ |
| 手柄拖动 | F | 抓取窗口 = **手指尺寸**（22pt 折算格子，`selectionHandleTolerance`）；抓中端点 → 拖动该端点；抓空 → 从 pivot 扩展 | U ✅ + M |
| 单指纵向拖 | B/C（primary） | 滚动本地 scrollback；到底自动释放 hold | S(待补)/M |
| 单指纵向拖 | E | wheel 事件转发给远端应用（tmux 由 `#{mouse_any_flag}` 决定 wheel vs copy-mode，见 scroll 架构 memory） | M |
| 单指纵向拖 | F | **让步给选择 pan**（app pan 不启动） | T ✅ |
| 单指横向拖 ≥40pt | B/C | 切 pane/window（axis lock） | M |
| 双指捏合 | 任意 | 字号缩放并持久化（含 cancelled 提交） | M |
| 滚动后 2s 内单击 | D | 与 D 行为一致（旧「阅读窗口吞点击」已退役，无特殊情况） | U ✅ |
| 键盘 toggle | 任意 | 唯一的显式聚焦入口；外接键盘时=纯聚焦不弹软键盘 | M |
| D-pad / scroll thumb | 任意 | 需按住停顿才激活（防背景手势误触发方向键——见 CHANGELOG「swipe away」修复） | M |

## 已知让步链（改手势前必读）

1. app `positionTap` **等待 double/triple tap 失败**（否则双击的第一击就发 click）。
2. app 单指 pan 在 `selection.active` 时 **shouldBegin=false**（选择 pan 独占拖动）。
3. fork singleTap 未聚焦分支：链接 → 打开并 return；否则 `focusOnTap`（Moshpit=false）→ 什么都不做。键盘判定完全归 app 的 `handleTap`。
4. `shouldRecognizeSimultaneouslyWith` 全局返回 true（app delegate）——防冲突靠 1/2 的显式让步，不靠互斥。
5. 选择 pan 是 fork 动态装卸的（enable/disableSelectionPanGesture），没有 delegate——它的“独占”由 2 保证。

## 待补清单（按价值排序）

1. **S 层手势 harness**（`scripts/verify-touch.sh`）：seed localhost → 按矩阵脚本化
   idb 点/拖 → 断言 `Log.input` 决策日志 + AX 树（键盘 up/down、Copy 菜单出现）。
   单击类已可全自动（决策日志就绪）；双击用两次快速 idb tap，需实测间隔是否 <300ms。
2. T 层补：选择激活时 pan 让步的直测（需要可注入的 selection 状态或 fork 暴露 test seam）。
3. 链接点击的 S 层（OSC-8 输出 + tap + 断言 openURL 被调——需要 app 侧 hook 日志）。

## 变更记录

- 2026-08-27 初版。当日修复：focusOnTap（fork 15）、按落点分键盘
  （`tapWantsKeyboard`，commit 3c931cb）、手柄手指化 + UIEditMenuInteraction
  （fork 16）、未聚焦态点击取消选中（fork `closeSelection` + app handleTap）、
  长按=选词而非弹键盘（fork 17）。
