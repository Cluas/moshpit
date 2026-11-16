<!-- source: README.md 2d4fb4a3e4f6551abe6e997954516d14aadde7dd -->
<div align="center">

<img src="Moshpit/Resources/AlternateIcons/Previews/IconPreview-Moshpit@3x.png" width="112" alt="Moshpit 的 App 圖像：一顆像素蛋從終端機窗格後面探出頭">

# Moshpit

### 智能體在你的機器上寫程式，你在手機上回它。

**Claude Code、Codex、Gemini 在伺服器上照常工作；哪一個需要你，手機就知道。**<br>
iPhone 和 iPad 上的 SSH、Mosh、tmux、herdr 終端機：每個提問都會送來一則加密推播，<br>
即時動態隨時顯示誰在等你、誰在工作、各自多久了。

[![App Store](https://img.shields.io/badge/App_Store-iPhone_%26_iPad-111111?logo=apple&logoColor=white)](https://apps.apple.com/app/id6799896801) ![iOS and iPadOS 18+](https://img.shields.io/badge/iOS_%7C_iPadOS-18%2B-111111?logo=apple&logoColor=white) [![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-111111)](LICENSE)

**[App Store](https://apps.apple.com/app/id6799896801)** &nbsp;·&nbsp; **[官網（簡體中文）](https://moshpit.cluas.eu.org/zh)** &nbsp;·&nbsp; **[文件（簡體中文）](https://moshpit.cluas.eu.org/zh/docs)** &nbsp;·&nbsp; **[Issues](https://github.com/Cluas/moshpit/issues)**

[English](README.md) · [简体中文](README-zh.md) · **繁體中文** · [日本語](README.ja.md) · [한국어](README.ko.md) · [Deutsch](README.de.md) · [Español](README.es.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md)

<br>

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/zh-Hant/01-lock.png" width="250" alt="誰在等你、等你什麼，鎖定畫面就看得到"> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/zh-Hant/03-home.png" width="250" alt="誰在跑、誰在等你，一眼就知道"> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/zh-Hant/02-pane.png" width="250" alt="點開，直接回到發問的那個窗格">

</div>

<br>

## Moshpit 適合你嗎？

如果你在一台 SSH 連得到的機器上（工作站、伺服器、雲端主機）跑 Claude Code、Codex、Gemini CLI 這類寫程式的智能體，而且常常開在 tmux 或 herdr 裡，又希望哪個智能體發問時直接用手機回，不必走回筆電前面，那 Moshpit 就是為你做的：

- **需要你的時候，你會知道。** 智能體一提問，鎖定畫面上就會出現一則推播：在你的主機上加密，到手機上才解開；點一下就直接進到發問的那個窗格。
- **所有智能體一次看完。** 首頁先列出所有主機上「在等你」的智能體，接著是誰在工作、做了多久，最後是誰閒置。
- **真正的終端機。** Mosh 換網路也不斷線，tmux 的視窗和窗格以原生方式顯示和管理，有快速鍵列，還能直接顯示圖片。

如果你的智能體不在你連得到的機器上跑，那它就不適合你：Moshpit 是終端機，不是跑智能體的地方，也不是託管型智能體的聊天 App。目前只有 iPhone 和 iPad 版；Android 版還在開發，尚未推出。

## 快速開始

**1 · 安裝。** 在 [App Store](https://apps.apple.com/app/id6799896801) 購買 Moshpit（一次買斷），或是[自己從原始碼建置](#從原始碼建置)。

**2 · 新增主機。** 填入位址和使用者名稱，登入方式選密碼或 SSH 金鑰：金鑰可以在手機上產生、從別處匯入，或直接貼上 PEM。任何 SSH 伺服器都能連。

**3 · 設定主機（選用）。** *設定主機*會安裝回報各智能體狀態的狀態腳本，並替這台主機完成配對，才收得到推播。在主機上改動任何東西之前，它都會先問過你。Codex 還要在主機上多做一步：在 Codex 裡執行 `/hooks`，信任 Moshpit 的那四個項目。

**4 · 加裝 Mosh、tmux 或 herdr（選用）。** Mosh 讓工作階段在切換網路時不中斷；tmux 和 herdr 的窗格會以原生方式呈現。App 裡每一樣都附有安裝指南。

## 支援的智能體

| 智能體 | 狀態 | 在等你 | 已完成 |
| :-- | :-: | :-: | :-: |
| Claude Code | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ ¹ | ✓ |
| Gemini CLI | ✓ | ✓ ² | ✓ |
| Qwen Code | ✓ | ✓ | ✓ |
| Qoder | ✓ | ✓ | ✓ |
| Factory（`droid`） | ✓ | ✓ | ✓ |
| CodeBuddy | ✓ | ✓ | ✓ |

**狀態**顯示在首頁、動態島和即時動態裡；**在等你**和**已完成**是推播。¹ Codex 在要權限時發問。² Gemini CLI 只在工具要權限時發問。

*設定主機*只會替它在主機上找到的智能體安裝狀態腳本。用 tmux 時，首頁上的狀態來自這些狀態腳本；用 herdr 時，狀態由 herdr 自己回報，完全不用設定。推播則不管智能體在哪裡跑，都由狀態腳本送出：tmux 裡、herdr 裡，或一般的 shell 裡都一樣。

「在等你」推播會先等 30 秒，所以你在電腦前就回掉的提問，不會再跑到手機上。「已完成」推播會寫出這一輪跑了多久；主機上有 `jq` 的話，還會寫出它回應的是哪一則提示詞。跑滿三分鐘的會響提示音，比較短的則安靜送達。

## 各部分怎麼串在一起

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/architecture-dark.svg">
  <img src="docs/assets/architecture-light.svg" width="900" alt="Moshpit 的架構：iPhone 和 iPad 上的 App（薄薄一層 Swift 外殼，底下是 Rust 核心）透過 SSH 或 Mosh 連到你的主機，讀取 tmux 或 herdr 的窗格和每個智能體的狀態。在主機上，智能體的狀態腳本把狀態標記在窗格上，並把「在等你」和「已完成」交給一支負責加密的腳本；加密後的推播先經過只看得到密文的推播服務，再經過 APNs，送到手機上的通知服務，在那裡解開。">
</picture>

App 透過 SSH 或 Mosh 連到你的主機，用 tmux 的控制模式驅動 tmux，用 herdr 自己的協定驅動 herdr，所以窗格是原生繪製的，每個智能體的狀態也是以資料的形式傳過來，而不是從輸出內容去猜。在主機上，每個智能體的狀態腳本會呼叫一支小腳本：把智能體的狀態標記在窗格上；遇到「在等你」和「已完成」時，再用*設定主機*時配對好的金鑰把推播加密。推播服務只負責把加密好的信封轉送給 APNs，看到的只有密文；手機上的通知服務再把它解開。完整的推播協定請見 [docs/PUSH.md](docs/PUSH.md)（英文）。

## 能做什麼

### 首先是終端機

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/zh-Hant/05-overview.png" width="220" alt="tmux、herdr 的窗格，一個畫面全看到"> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/zh-Hant/08-keyboard.png" width="220" alt="專為終端機設計的鍵盤">
</p>

| 功能 | 說明 |
| :-- | :-- |
| **SSH 和 Mosh** | Mosh 附預測回顯。Wi-Fi 和 5G 之間切換、手機休眠、飯店的爛網路，工作階段都不會斷；App 在背景待了一陣子，只要連線還在，回來就直接接著用，不必重新連線 |
| **連不上時會說原因** | 是連接埠拒絕連線、主機名稱解析不到（或沒開區域網路權限）、主機沒有回應（檢查一下 VPN），還是從目前的網路根本沒有路由到得了它 |
| **tmux 控制模式** | 原生繪製，視窗和窗格都能新增、重新命名、關閉。不是把一個縮小的假桌面硬塞進手機 |
| **[herdr](https://github.com/herdrdev/herdr)** | 專為寫程式的智能體打造的終端機多工器，一次顯示一個全寬的窗格 |
| **窗格總覽** | 一個工作階段裡的所有視窗和窗格一次攤開，每一格都是該窗格畫面的即時影像，tmux 和 herdr 都一樣。左右滑動就能在窗格之間切換 |
| **tmux 不在了的時候** | tmux 伺服器被強制結束或當掉時，首頁不會再把它的工作階段和智能體當成還活著，終端機會直接提供「新增工作階段」 |
| **用 Metal 繪製** | 核心是我們 fork 的 wezterm 終端機：能直接顯示圖片（kitty、iTerm2、sixel），能在捲動歷史裡尋找，選取範圍附拖移控點，往回捲動歷史時保有慣性 |
| **輸入** | 快速鍵列提供 esc / ctrl / alt / 方向鍵，支援輸入法組字，連結點了就開，可自訂主題，內建七種終端機字體 |
| **主機** | 每台主機都有自己的頭像，由它的主機金鑰畫出來（同一把金鑰永遠是同一張臉），也可以換成你挑的表情符號。用備份還原到新 iPhone 後，Moshpit 會列出哪些密碼和金鑰還留在舊手機上（鑰匙圈項目從來不會跟著搬過去），讓你重新輸入或匯入；你信任過的主機金鑰則依然受信任 |

### 為智能體的工作而設計

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/zh-Hant/04-island.png" width="220" alt="不用打開 App，進度就在動態島上">
</p>

| 功能 | 說明 |
| :-- | :-- |
| **「在等你」排第一** | 首頁最上面列出所有主機上在等你的智能體。往下是誰在工作、做了多久，以及誰閒置 |
| **推播直接上鎖定畫面** | 端對端加密，App 沒開也送得到。點一下就進到發問的那個窗格 |
| **即時動態** | 顯示在鎖定畫面和動態島上：誰在等你、誰在工作、各自多久了 |
| **開一個新的工作樹** | 在手機上讓智能體在新的 git worktree 裡開工：選一個儲存庫、取個分支名稱，也可以順便交給它第一則提示詞 |
| **分享到窗格** | 從任何 App 分享圖片，直接送進智能體的窗格 |

### 智能體提到的檔案

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/zh-Hant/06-preview.png" width="220" alt="改了哪個檔案，點一下就能看">
</p>

點一下智能體輸出裡的路徑，檔案就會在手機上打開：圖片可以預覽，影片可以播放，文字就照文字讀。打開後還能瀏覽它所在的資料夾，或把檔案分享出去。

### 語音

聽寫在裝置上完成。**Qwen3-ASR**（0.6B，下載一次就好）能把夾著英文術語的中文當成同一句話聽寫，還會把窗格畫面上的檔案名稱、分支和智能體名稱當成熱詞。也可以用 **Whisper** 和 Apple 的裝置端辨識引擎。音訊不會離開手機。

### iPad

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/ipad/zh-Hant/02-home-terminal.png" width="300" alt="首頁和終端機並排">
</p>

左邊是主機，右邊是終端機。接上硬體鍵盤時，<kbd>⌘K</kbd> 顯示所有窗格，<kbd>⌘1</kbd>–<kbd>⌘9</kbd> 切換視窗，<kbd>⌘W</kbd> 回到首頁。

### 夥伴和 App 圖像

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/zh-Hant/07-pet.png" width="220" alt="孵一隻陪你等的像素夥伴">
</p>

| 功能 | 說明 |
| :-- | :-- |
| **像素夥伴** | 在首頁從蛋裡孵出來，停在最需要你的那個智能體的卡片上，跟著它的狀態變化，也會在動態島和即時動態上陪著你。開啟後，每台主機的頭像也會變成一隻像素小怪，長相同樣由主機金鑰決定 |
| **22 個 App 圖像** | 全都在設定裡：預設是一顆像素蛋；六種夥伴各有一個從窗格後面探出頭的圖像（每種另有一個閃光版），另外還有 Classic 和其他八個。夥伴孵化後，首頁會問你要不要換成牠那一種 |

這裡的截圖都來自 App Store 用的那一套，由 Screenshots 工作流程產生，每次完整跑完就會更新。所有語言和 iPad 的截圖：[全部截圖](https://blog.cluas.eu.org/moshpit/shots/#zh-Hant)。

## 不玩花樣

- 一次買斷。沒有訂閱，不用帳號，也沒有功能分級。
- 不蒐集任何資料：沒有分析，沒有追蹤。金鑰留在 iOS 鑰匙圈裡，流量只到你自己的伺服器，再加上一個只看得到密文的推播服務。
- Moshpit 不會沒問過你就在主機上安裝東西，也不會背著你開工作階段；連線降級時會直接告訴你，而不是悄悄換一種傳輸方式。

## 怎麼做出來的

Moshpit 大部分是用 Rust 寫的。核心在 `crates/`：SSH 傳輸層（`ssh`，建立在 russh 上）、Mosh 的協定（`mosh`）、負責撥號、重新連線和恢復的工作階段層（`session`）、tmux 控制模式（`tmux`）和 herdr（`herdr`）、終端機模擬器（`term`，也就是 `vendor/wezterm` 裡我們 fork 的 wezterm）、App 要保存的資料（`store`）、智能體的狀態機（`agents`）、夥伴（`pet`），以及主機設定和推播配對（`hostcare`、`install`、`seal`）。畫面也是用 Rust 寫的，靠的是 [NSH](https://nsh.dev)，放在 `ui/`，畫面背後的模型則在 `crates/app`。

在 iOS 上，薄薄一層 Swift 透過一個靜態函式庫（`crates/ffi`）承載這一切：App 外殼、終端機檢視的 Metal 繪製和文字輸入、鑰匙圈、聽寫（Qwen3-ASR 跑在 sherpa-onnx 的 onnxruntime 上，Whisper 跑在 WhisperKit 上，再加上 Apple 的語音辨識引擎），還有 iOS 只讓原生程式碼經手的部分（即時動態和小工具、透過 `crates/seal-ffi` 解開推播的通知服務、分享延伸功能）。Android App 共用同一套核心和畫面（`crates/android`、`android/`），還在開發中，尚未推出。

<details>
<summary><b>儲存庫結構</b></summary>

<br>

| 路徑 | 裡面有什麼 |
| :-- | :-- |
| `crates/` | Rust 核心，一件事一個 crate：`ssh`、`mosh`、`session`、`tmux`、`herdr`、`term`、`stream`（窗格的位元組在核心和終端機檢視之間的通道）、`store`、`agents`、`pet`（夥伴）、`seal`（推播的信封格式）、`seal-ffi`（通知服務呼叫它的唯一入口）、`hostcare`、`install`、`app`（各個畫面的模型）、`runtime`、`copy`（中文文案要守的規則）、`ffi`（iOS 連結的靜態函式庫），以及 `android`（Android App 的 Rust 那一半） |
| `ui/` | 用 Rust 和 NSH 寫的畫面，iOS 和 Android 共用 |
| `vendor/wezterm/` | 我們 fork 的 wezterm 終端機模擬器，接在 `crates/term` 後面；改了哪些地方列在 [docs/WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md) |
| `Moshpit/` | iOS 這一層：App 外殼、每個畫面的 Swift 部分、終端機檢視的 Metal 繪製和文字輸入、鑰匙圈、聽寫（Qwen3-ASR、Whisper、Apple 的辨識引擎） |
| `Extensions/` | 三個 App 延伸功能：`MoshpitIsland/`（即時動態和鎖定畫面小工具，Vibe Island）、`MoshpitPush/`（在裝置上解密推播的通知服務）、`MoshpitShare/`（把圖片交給智能體的窗格） |
| `Packages/MoshpitKit/` | App 和三個延伸功能都會連結的本機 Swift 套件：記錄、即時動態的資料內容、加密推播的信封格式和配對儲存 |
| `android/` | Android App（Gradle），開發中 |
| `Tests/` | `MoshpitTests/` 單元測試，`MoshpitUITests/` UI 測試 |
| `push-relay/` | 你的主機和 APNs 之間那個不保存狀態的 Go 推播服務，部署設定檔在 `deploy/` 底下 |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md)、[PUSH.md](docs/PUSH.md)（推播協定從頭到尾）、[WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md)、`nsh/`（Rust 核心和 NSH 畫面的設計筆記）、其他設計筆記 |
| `scripts/` | `rust/` iOS 函式庫的建置和它的 Swift 綁定，`host/` 裝到你開發主機上的 shell 腳本，`gen/` 產生器，`verify/` 端對端檢查，`spikes/` 實驗，`wezterm/` 從上游匯入 fork 的更新，`release/` 封存打包工具 |

</details>

App 預設使用維護者架設的推播服務；它看得到什麼、看不到什麼，以及怎麼自己架一個，請見 [docs/PUSH.md](docs/PUSH.md)。

## 從原始碼建置

你需要裝了 Xcode 27 的 macOS（App 的部署目標是 iOS 18）、用 `brew install xcodegen` 安裝的 [XcodeGen](https://github.com/yonaskolb/XcodeGen)，以及透過 [rustup](https://rustup.rs) 安裝的 Rust。儲存庫在 `rust-toolchain.toml` 裡固定了工具鏈版本，rustup 第一次用到時會自動裝好對的版本。

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # project.yml 才是唯一的來源，.xcodeproj 是由它產生的
open Moshpit.xcodeproj
```

選 **Moshpit** scheme 和一台模擬器，按 ⌘R。App target 的「Build Rust」階段會執行 `scripts/rust/build-ios.sh`，把 `crates/ffi` 建置成給模擬器或實機用的 `libmoshpit_ffi.a`，第一次還會順便加上需要的 Rust target（`aarch64-apple-ios-sim`、`aarch64-apple-ios`）。Debug 建置用 cargo 的 `app-test` profile，Release 建置用 `app`。第一次建置要把整套 Rust 相依套件都編譯一遍，會花上一段時間；之後都是增量編譯。Swift 套件也會在第一次建置時解析下載。

實機建置需要在 `Signing.xcconfig` 填入你的 Team ID；那個檔案裡的註解和 [docs/install-free-account.md](docs/install-free-account.md) 會一步步帶你設定，包括用免費 Apple ID 的做法。

### 測試

Rust 這邊，跟 CI 的跑法一樣：

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p moshpit-term        # 只測一個 crate
```

iOS App，跟 CI 在 Xcode 27 映像檔上的跑法一樣：

```sh
scripts/rust/test-build-ios.sh    # 只檢查 Rust 階段用的 profile 和連結設定，不實際建置
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' CODE_SIGNING_ALLOWED=NO
```

有幾組測試在沒有簽署的建置上會跳過（它們需要 App Group 容器），有幾組不在 iPad 模擬器上就會跳過（硬體鍵盤的快速鍵），還有一些 UI 流程在針對目前的畫面重寫之前也會先跳過；記錄裡會寫明是哪幾個。

Android App 用 `android/ci.sh` 建置和測試；只要有 JDK 17，其他需要的東西（SDK 元件、NDK、Gradle）它都會自己抓。

## 參與貢獻

工作流程和專案慣用的寫法請看 [CONTRIBUTING.md](CONTRIBUTING.md)，回報任何敏感問題之前請先讀 [SECURITY.md](SECURITY.md)。這裡的每個人都遵守[行為準則](CODE_OF_CONDUCT.md)。

Moshpit 有分享在 [LINUX DO](https://linux.do)，那是一個中文開發者社群。有問題或點子，在那邊提和在 issue tracker 提一樣歡迎。

## 授權

Moshpit 是自由軟體，以 [GNU 通用公眾授權條款 v3.0](LICENSE)（GPL-3.0-only）發布。你可以依照這份授權條款建置、修改和再散布它。App Store 上的版本由持有著作權的維護者發布。第三方元件各自保有原本的授權，列在 [NOTICES.md](NOTICES.md)。
