# Moshpit 用 NSH 时碰到的缺口

这份清单给 NSH 维护者看：Moshpit 把 iOS 的每一屏搬到 NSH（W11a–W11h，2026-11-11/12）时，NSH 做不到、只能由 App 自己补的地方。每条写现象、Moshpit 现在的绕法，和希望 NSH 提供什么。按影响排序。还没有提给上游，提不提由 Cluas 决定。

## 影响大的

1. **没有原生的表单和浮层容器。** NSH 没有底部表单的停靠高度（detent），也没有带箭头的浮层（popover）。Moshpit 让宿主用系统容器装 NSH 内容（选择器、安装协助、主机文件夹）。希望：NSH 能自己弹出表单（含停靠高度）和浮层，iPad 上从指定元素挂下来。
2. **没有原生长按菜单。** `nsh_base` 的 `context_menu` 在手机上是底部弹出的表单，不是系统菜单；原生组件只能是叶子节点，包不住 NSH 内容。Moshpit 在整个 NSH 视图上挂一个 `UIContextMenuInteraction`，再按无障碍标识找到被按的行（`NSHContextMenus`）。希望：任意 NSH 元素可以挂一个系统长按菜单。
3. **没有系统样式的控件。** 开关、分段控件、滑块、菜单按钮、进度条、取色器都没有平台原生版本。Moshpit 注册了宿主组件 `moshpit/switch`、`segmented`、`slider`、`menu`、`progress`、`color-well`。希望：像 TabBar 一样提供平台原生控件。
4. **键盘动画和键盘不同步。** 键盘曲线的动画在收到通知时就开始，比键盘真正开始动早约 30 ms，快捷键条会先冲出去。Moshpit 的 `NSHKeyboardAlign` 按键盘自己的开始时间重启动画（领先 30.0 → 0.0 ms）。希望：`LayoutCurve::Keyboard` 对齐键盘自己的动画。
5. **文字度量和 UIKit 不一样。** 文字框向上取整到整点、行在行高里居中；UIKit 取整到像素、多出的空间在上方、行距只加在行与行之间。Moshpit 给单行文字定高并下移、多行文字加边距再扣掉多出的部分。希望：提供按 UIKit 方式度量文字的选项，和按基线对齐的接口。
6. **换行规则不一样。** 没有「避免末行孤词」，连字符后会断行。Moshpit 用不换行空格把最后两个词连起来、连字符后加词连接符。希望：提供和 TextKit 一致的换行策略。

## 影响中等的

7. **弹窗没有输入框。** `AlertDialog` 不能带文字输入框。Moshpit 让宿主弹 UIKit 的输入提示（`NSHTextPrompt`）。
8. **iPad 上的操作表挂错位置。** 操作表固定从屏幕底部中间弹出、没有箭头。Moshpit 的 `NSHDialogAnchor` 在 UIKit 布局前把它挪到触发元素上。希望：操作表能指定锚点。
9. **导航栏标题不能挂菜单，栏按钮不能挂菜单。** Moshpit 把宿主的菜单按钮盖在 NSH 画的标题或按钮上。
10. **没有列表编辑模式。** 删除用的红色 −、拖动排序手柄都要自己画，拖动用 PanResponder。
11. **没有存密文的输入框。** 私钥和口令只能用宿主组件输入，这样内容也不经过 NSH 引擎——这一条 Moshpit 反而想保持。
12. **输入框的默认值只生效一次。** 区域重建时节点被复用，默认值不再更新。Moshpit 换 key 强制重建输入框（`form::remount`）。
13. **原生组件的 `test_id` 落在 NSH 的容器上**，不在组件本身。Moshpit 把标识当属性传进去。
14. **模糊材质跟着背景色、不跟外观。** 深色材质在浅背景上会变浅灰。
15. **没有退场动画、没有模态的无障碍隔离。** 卡片消失是瞬间的；卡片显示时 VoiceOver 还能摸到后面的内容。Moshpit 自己隐藏后面的内容、等淡出结束再卸载。
16. **iOS 上不能按文件路径加载图片**，只能用 `asset:/`。Moshpit 把资源目录设成 App 包，缩略图用宿主组件。

## 影响小的

17. 边框计入盒子尺寸（SwiftUI 的描边画在上面）；没有负外边距；给定高的单行文字加内边距不会变高。
18. 没有按子树限制动态字号范围（SwiftUI 的 `dynamicTypeSize`）。
19. 没有焦点 API 给原生组件；没有在窗口坐标里量一个视图的接口（Moshpit 用 `ElementRef.measure` 报给宿主）。
20. 没有径向渐变（带圆心和半径）、路径或折线绘制。
21. 无界面（headless）测试里不能触发原生事件，图片、原生组件的尺寸、像素都只能上真机看。
22. Mac Catalyst 上 `on_key` 先于宿主的快捷键命令拿到 ⌘ 键，而菜单栏的快捷键不管按键被谁处理都会再执行一次——和菜单项撞键的绑定会触发两次（⌘W 实测）。希望：Catalyst 上按键表跳过菜单栏占用的键。
23. 输入框的 `max_length` 按 UTF-16 计数，不按字素：4 个 emoji 只能输 3 个；`shouldChangeCharactersInRange` 在拼音组字中途是否也会拦，未核实。Moshpit 的快捷键标签和主键（上限 6）受影响，去掉上限会显示比草稿多的字、中途改写又丢焦点。希望：按字素的 `max_length`，或者受控输入。

25. **`nsh_base` 的按钮只报 `selected`，报不了 `busy`、`expanded`。** 读屏听不到「正在处理」的按钮、展开收起的行。Moshpit 的 `ui` 层给展开行（`ui::row`，R3）和忙着的栏按钮（`nav_bar::BarContent::Busy`，R15/R16）各做了自己的按下目标（`view().role(Button)` 加 `accessibility_state`），代价是这两种不走 `nsh_base` 按钮的工具栏和键盘焦点规则。希望：`nsh_base::button` 能传整个 `AccessibilityState`。

## 缺陷（不是缺口）

下面是 NSH 本身的错，不是少了什么。行号按发现时钉住的版本 `6a4f1de19c9275b7f185fc767b1451a1b1cef27f`。

24. **iOS 上，滚动视图里后来才建出原生视图的节点，丢了样式里的 `pointer_events`。** **已在上游修好：`b37c601b`（longbridge/nsh#1812，11-12）；Moshpit 自 `71cf4715` 起钉住含这个修复的 `53b2cfa4`。** 现象：主机文件夹的行卡片上盖着一层描边（`absolute().inset_0()…pointer_events(PointerEvents::None)`，和行是兄弟、画在行的上面），真机上点任何一行都没反应（B302）；同一页面在无界面测试里按得到。经过：
    - iOS 的界面开着内容裁剪（`nsh-platform-ios/src/surface.rs:751`，`content_culling: true`）：滚动视图里的内容等靠近视口时才由裁剪那一步建原生视图（`nsh-core/src/core/cull.rs:330` → `ensure_created`，`core/mod.rs:6384`）。无界面测试把裁剪关了（`nsh-testing/src/headless/mod.rs:193`），所以看不到。
    - 样式里的 `pointer_events` 算出来存在节点的 `class_pointer` 上（`core/classes.rs:1544–1592`），算完给节点标 `VIEW_PROPS`（`classes.rs:1588–1591`）——这时它的原生视图可能还没建。
    - `flush_dirty_once` 先把节点的脏标记取走，再对还没建的节点直接跳过（`core/mod.rs:6637`、`6647`）：这个 `VIEW_PROPS` 就这样丢了。
    - 后来裁剪那一步建视图时，`mount_state` 只在 `node.props.is_some()` 时才补发属性（`core/mod.rs:6551–6557`）；只有样式里的 `pointer_events`、没有别的属性的节点，`node.props` 是空的，于是一条 `ViewProps` 都不发，原生 `NshView` 一直是默认的 `PointerEvents::Auto`，命中测试照常把触摸给它（`nsh-platform-ios/src/native_view.rs:157–170`）。
    - 给它加任何一个属性（比如 `.test_id(…)`）`node.props` 就不空了，属性会发，`mounted_props` 再补上 `class_pointer`（`core/mod.rs:832–834`）——真机上加个测试标识它就好了，就是这个原因。
    - 不只 `None`：`BoxNone`、`BoxOnly` 走的是同一条路，裁剪范围里没有别的属性的节点都会丢（读代码可见，没有单独测）。
    
    最小复现（无界面，照 iOS 打开裁剪）：`Headless::start` 之后 `app.engine().control().set_content_culling(true)`，跑一个滚动视图，里面是一张卡片和一层 `absolute().inset_0().pointer_events(PointerEvents::None)` 的兄弟描边，再 `press` 卡片里的按钮：报 `is covered: a press there reaches view:…`，`MountBatch` 里那层描边没有任何 `ViewProps`；同样的东西关掉裁剪，`ViewProps { pointer_events: None }` 有、按得到。真机复现：同一层描边在没有测试标识时盖住所有行，加上 `.test_id` 就不盖。
    
    **由此得出的规矩（Moshpit 的代码）：** iOS 上，滚动视图里一个视图如果除了样式里的 `pointer_events` 再没有别的属性（测试标识、无障碍标签、角色、`accessibility_elements_hidden` 这些都算属性；背景、描边、透明度是样式，不算），它就不能盖在可按的东西上面——它的 `pointer_events` 到不了原生视图，照样接触摸。能做的：画在可按东西的后面（排在它前面的兄弟），或者画在可按视图自己里面（触摸落到它身上，再往上交给可按视图的手势）。钉住修好的版本以后，这条不再是必须遵守的，但装饰画在可按东西后面仍然是更稳的写法。
    
    11-13 对照这条规矩查了一遍 `ui/src` 里滚动视图中的 12 处 `pointer_events(None/BoxNone)`：6 处画在它装饰的可按视图里面（设置和添加快捷键的色块选中圈、App 图标的描边、文件夹缩略图的描边、首页树里当前会话的圈、首页卡片的状态条）；2 处另有属性，`pointer_events` 会发出去（添加密钥的占位文字、快捷键确认时的占位按钮）；文件夹行卡片的描边已经挪到行后面（`75a4cab0`）；剩下 3 处是 1 像素的分隔线和两个 1 pt 高的滚动锚点，最多压住上一行最底下一像素。另外把全部无界面测试临时按 iOS 打开裁剪跑了一遍：没有一处按下报 `is covered`（失败的 9 个都是设置页折叠以下的行还没建、`wait_for` 等不到，不是被盖住）。所以今天没有被盖住的可按视图；裁剪默认仍然关着，只在文件夹的测试里打开。
    
    Moshpit 现在的做法：不靠 `pointer_events(None)` 让开触摸。装饰画在可按的东西后面（文件夹的描边改成画在行的下面，`75a4cab0`），或者画在可按的视图里面；每个 NSH 页面的真机测试都用 `NSHTouchAudit` 在每个可按视图的中心做一次 UIKit 的命中测试，盖住了就失败；文件夹的无界面测试打开裁剪按每一行。上游的修法就是前面说的那条：新视图的 `class_pointer` 不是 `Auto` 时，`mount_state` 也发它的属性。回归测试 `ui/tests/nsh_pointer_events.rs`：按 iOS 的样子打开裁剪，列表里的按钮上各盖一层只有样式 `pointer_events`（`None`、`BoxNone`）的描边，逐个去按；在 `6a4f1de` 上报 `` `id:button-none` is covered: a press there reaches `view:15` ``，在 `53b2cfa4` 上按得到。还剩的缺口：无界面测试默认仍然不裁剪，要测得自己打开。
