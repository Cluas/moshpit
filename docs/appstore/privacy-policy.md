# Moshpit Privacy Policy

*Effective: ⟨发布日期⟩ · Contact: ⟨支持邮箱⟩*

Moshpit is a terminal. Your data is none of our business, and the app is
built so it never becomes our business.

## What we collect

**Nothing.** Moshpit has no analytics, no crash reporting service, no
advertising SDKs, no tracking, and no accounts. We run no servers that your
data could be sent to.

## Where your data lives

- **Connection details** (hosts, usernames, ports) are stored on your device.
- **Passwords and private keys** are stored in the iOS Keychain, protected by
  the Secure Enclave where available. They never leave your device except as
  part of the SSH/Mosh handshake with the server *you* configured.
- **Terminal content** exists only in transit between your device and your
  servers, encrypted by SSH or Mosh (AES-128-OCB3), and on your screen.

## Network connections

Moshpit connects exclusively to servers you add. There are no first-party or
third-party endpoints: no telemetry, no update pings, no font/CDN fetches.
The Local Network permission is requested only so SSH/Mosh can reach hosts on
your own LAN (e.g. 192.168.x.x); iOS would otherwise silently drop that
traffic.

## Notifications & Live Activities

Agent-status notifications and the Dynamic Island Live Activity are generated
**on your device** from your own session data. They use no push servers; we
could not see them if we wanted to. The lock-screen preview of what an agent
is asking can be turned off in Settings.

## Purchases

Moshpit is a one-time purchase handled entirely by Apple's App Store. We
receive no payment information.

## Changes

If this policy ever changes, the new version ships with an app update and is
dated at the top. Since the app collects nothing, the realistic change is
wording, not substance.

---

# 隐私政策（中文）

Moshpit 是一个终端。**我们不收集任何数据**：无分析、无崩溃上报服务、无广告
SDK、无跟踪、无账号，也没有任何我们自己的服务器。

- 连接信息存于你的设备；密码与私钥存于 iOS 钥匙串（有 Secure Enclave 则由其
  保护），除与你自己配置的服务器做 SSH/Mosh 握手外绝不离开设备。
- 终端内容只在你的设备与你的服务器之间加密传输（SSH / Mosh AES-128-OCB3）。
- App 只连接你添加的服务器；本地网络权限仅用于访问局域网主机。
- 通知与灵动岛均在设备本地生成，不经任何推送服务器。
- 购买由 Apple App Store 处理，我们接触不到任何支付信息。

联系方式：⟨支持邮箱⟩
