# 本地安装 Ringdown 到 iPhone(免费 Apple 开发者账号)

免费 Apple ID 足以把 Ringdown 装到你自己的 iPhone——本项目 entitlements 为空、无
App Groups / 推送,Live Activity 只是个 Info.plist 开关,全部免费档可用。

## 免费账号能做 / 不能做

| ✅ 可以 | ❌ 不行 |
|---|---|
| 装到自己配对过的 iPhone | TestFlight / 对外分发 |
| mosh + SSH + tmux 全功能 | 后台远程推送(我们不用) |
| 本地通知 + 灵动岛 Live Activity | App 不在同一网络时远程安装 |

**硬限制(记住)**:① App **7 天过期**,过期后图标在但打不开,重新 ⌘R 一次即可续(数据保留)。② 同时最多 **3 个**自签 App。③ 每 7 天最多 10 个 App ID(Ringdown 占 2 个:主 App + 灵动岛扩展)。

---

## A. 一次性准备(在 Mac 上,手机可不在场)

1. **加 Apple ID**:Xcode ▸ Settings ▸ Accounts ▸ 左下 `+` ▸ Apple ID ▸ 登录。
   登录后会自动出现一个 **"(Personal Team)"**。

2. **拿到 Team ID**(10 位):
   ```bash
   ./scripts/team-id.sh
   ```
   若提示还没有 team:先在 Xcode 里打开 `Ringdown.xcodeproj`,选 **Ringdown** target ▸
   Signing & Capabilities ▸ Team 下拉选你的 Personal Team(这一步会触发 Xcode
   创建证书),再跑一次脚本。

3. **填进签名配置**(让重新生成工程也不丢):
   编辑根目录 `Signing.xcconfig`,把 Team ID 填到 `DEVELOPMENT_TEAM =` 后面。
   然后:
   ```bash
   xcodegen generate
   git update-index --skip-worktree Signing.xcconfig   # 防止 Team ID 误入 git
   ```

> 为什么走 xcconfig:本项目用 xcodegen 生成 `.xcodeproj`,每次 `xcodegen generate`
> 都会重置工程——若只在 Xcode GUI 里选 Team,下次生成就丢了。写进 `Signing.xcconfig`
> 才持久。

---

## B. 装到手机(手机需在场:USB 线,或与 Mac 同一 Wi-Fi)

4. **手机开开发者模式**:设置 ▸ 隐私与安全性 ▸ 开发者模式 ▸ 打开 ▸ 重启。
   (iOS 16+ 首次部署时 Xcode 也会提示你开。)

5. **连接**:USB 接上,手机弹窗点"信任此电脑"。或同 Wi-Fi 下 Xcode ▸ Window ▸
   Devices and Simulators 里勾 "Connect via network"。

6. **运行**:Xcode 顶部设备选择器选你的 iPhone ▸ 按 **⌘R**。
   首次会自动注册设备、生成 Personal Team 的 provisioning profile。

7. **信任证书**:首次启动会被拦截 → 手机 ▸ 设置 ▸ 通用 ▸ VPN 与设备管理 ▸
   点你的开发者 App ▸ 信任 ▸ 回到 App 再启动即可。

---

## C. 命令行安装(可选,完成 B 的首次 GUI 运行之后)

首次务必用 Xcode GUI ⌘R(它负责注册设备 + 建 profile)。之后可纯命令行:

```bash
# 设备 UDID:xcrun devicectl list devices
xcodebuild -project Ringdown.xcodeproj -scheme Ringdown -configuration Debug \
  -destination 'generic/platform=iOS' \
  -derivedDataPath build/Device -allowProvisioningUpdates build

xcrun devicectl device install app --device <UDID> \
  build/Device/Build/Products/Debug-iphoneos/Ringdown.app
```

---

## D. 用 AltStore 自动续签(免去每周手动 ⌘R)

7 天过期是免费账号最烦的点。**AltStore 正是为此而生**:它用你的免费 Apple ID
侧载,并由常驻电脑的 **AltServer** 在后台、过期前自动重新签名,你无需每周手动重装。
好处:**这条路连 `Signing.xcconfig` 的 Team ID 都不用填**——签名由 AltStore 用它自己
登录的 Apple ID 完成。

1. **出包**(未签名 .ipa,含灵动岛扩展):
   ```bash
   ./scripts/build-ipa.sh        # → build/Ringdown.ipa(约 9 MB)
   ```
2. **装 AltStore**:Mac/PC 装 [AltServer](https://altstore.io),手机装 AltStore,
   在 AltStore 里用你的 Apple ID 登录。
3. **侧载**:把 `build/Ringdown.ipa` 拖进 AltStore(或 `altserver -u <UDID> build/Ringdown.ipa`)。
   AltStore 用你的 Apple ID 重签后安装。
4. **开自动续签**:AltStore ▸ Settings ▸ 打开 Background Refresh。让 AltServer 在
   电脑常驻,手机与电脑同一 Wi-Fi(并开启设备的 Wi-Fi 同步)即可后台续签。

**前提与现实**:
- AltServer 需要在一台电脑上常驻、且手机定期能与它同网可达——后台刷新才会发生;
  完全离线太久仍会过期(打开 App 触发一次前台刷新即可补上)。
- AltStore 本身占用免费账号 3 个 App 名额里的 1 个(剩 2 个;Ringdown 是其中之一)。
- 后台刷新不是 100% 可靠(iOS 后台调度 + 设备可达性所限),但比每周手动强得多。
- **想完全摆脱"电脑常驻"**:用 [SideStore](https://sidestore.io)(AltStore 分支,
  靠设备本地 WireGuard 回环续签,无需电脑在同一网络)。流程同上,出包脚本通用。
- **Tailscale 内网能替代"同一 Wi-Fi"吗?基本不能**。经典 AltServer 靠局域网
  Bonjour/mDNS(链路本地组播)发现设备,**组播不跨 Tailscale 的 L3 overlay**,所以
  AltServer 隔着 Tailscale 通常发现不了手机;即便手动指定 IP,iOS 的 lockdownd 也未必
  在 Tailscale 接口上可达。别指望它做后台续签。**续签请用 SideStore(设备本地,
  与网络无关)**。Tailscale 真正的用武之地是下一条——连你的服务器。

## 常见问题

- **"Failed to register bundle identifier"** → `com.cluas.ringdown` 全球唯一被占用了。
  改 `project.yml` 里 `com.cluas.ringdown` → 更独特的(如 `com.<你的标识>.ringdown`),
  连带把 `.island` 后缀的扩展 id 一起改,`xcodegen generate` 后重试。
- **灵动岛扩展签名报错** → 免费账号偶发。临时验证主 App:在 `project.yml` 的 Ringdown
  target `dependencies` 里去掉 `RingdownIsland` 那两行,generate 后只装主 App(会暂时
  失去 Live Activity);确认主 App 能装后再加回来排查。
- **装上后 7 天打不开** → 正常,重新 ⌘R 续签。
- **想人不在家也能更新** → 免费账号做不到;升级 $99/年开发者账号后走 TestFlight(90 天)
  或直接上架。
