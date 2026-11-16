<!-- source: README.md 6c585bea06aec83880bcb57346ee39d0aa3a9b2b -->
<div align="center">

<img src="Moshpit/Resources/AlternateIcons/Previews/IconPreview-Moshpit@3x.png" width="112" alt="Moshpit のアイコン：ターミナルのペインの向こうから顔を出すピクセルのたまご">

# Moshpit

### 自分のマシンで動くコーディングエージェントのためのターミナル

**Claude Code、Codex、Gemini はサーバーで作業を続けます。あなた待ちになったら、手元のスマホでわかります。**<br>
iPhone と iPad で SSH、Mosh、tmux、herdr。質問が来るたびに暗号化したプッシュ通知で知らせ、<br>
誰が待っていて誰が作業中か、それぞれどれだけ経ったかをライブアクティビティに出します。

[![App Store](https://img.shields.io/badge/App_Store-iPhone_%26_iPad-111111?logo=apple&logoColor=white)](https://apps.apple.com/app/id6799896801) ![iOS and iPadOS 18+](https://img.shields.io/badge/iOS_%7C_iPadOS-18%2B-111111?logo=apple&logoColor=white) [![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-111111)](LICENSE)

**[App Store](https://apps.apple.com/app/id6799896801)** &nbsp;·&nbsp; **[ウェブサイト（英語）](https://moshpit.cluas.eu.org)** &nbsp;·&nbsp; **[ドキュメント（英語）](https://moshpit.cluas.eu.org/docs)** &nbsp;·&nbsp; **[Issues](https://github.com/Cluas/moshpit/issues)**

[English](README.md) · [简体中文](README-zh.md) · [繁體中文](README.zh-Hant.md) · **日本語** · [한국어](README.ko.md) · [Deutsch](README.de.md) · [Español](README.es.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md)

<br>

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ja/01-lock.png" width="250" alt="誰が何を待っているか、ロック画面でわかる"> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ja/03-home.png" width="250" alt="誰が作業中で、誰が待っているか、ひと目で"> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ja/02-pane.png" width="250" alt="タップすれば、聞いてきたペインへ">

</div>

<br>

## Moshpit が合う人

SSH で入れるマシン（ワークステーション、サーバー、クラウド上のマシン）で Claude Code、Codex、Gemini CLI などのコーディングエージェントを動かしていて、たいてい tmux か herdr の中で使っている。質問してきたエージェントには、ノートパソコンの前まで戻らずにスマホから答えたい。そういう人のためのアプリです。

- **出番が来たらわかる。** 質問はプッシュ通知としてロック画面に届きます。ホスト上で暗号化され、スマホの中で初めて復号されます。タップ 1 回で、質問してきたペインに入れます。
- **すべてのエージェントをひと目で。** ホームには、すべてのホストを通して、まずあなた待ちのエージェント、次に作業中のエージェントとその経過時間、最後に待機中のエージェントが並びます。
- **本物のターミナル。** ネットワークが変わっても切れない Mosh、ネイティブに操作できる tmux のウィンドウとペイン、ショートカットバー、インライン画像。

エージェントが、あなたの接続できるマシンで動いていないなら、Moshpit は向いていません。Moshpit はターミナルであって、エージェントを動かす場所でも、ホスティングされたエージェントと話すチャットアプリでもありません。対応しているのは iPhone と iPad だけです。Android 版は開発中で、まだリリースしていません。

## クイックスタート

**1 · インストール。** Moshpit を [App Store](https://apps.apple.com/app/id6799896801) で入手するか（買い切りです）、[自分でビルド](#ソースからビルド)してください。

**2 · ホストを追加。** アドレスとユーザー名を入力し、ログイン方法でパスワードか SSH キーを選びます。キーはスマホ上で生成しても、インポートしても、PEM を貼り付けてもかまいません。SSH サーバーなら何にでも接続できます。

**3 · ホストの設定（任意）。** *ホストを設定* は、各エージェントの状態を報告するフックをインストールし、プッシュ通知を受け取れるようにホストとペアリングします。ホスト上の何かを変更する前には、必ず先に確認を求めます。Codex だけはホスト側でもうひと手間かかります。Codex で `/hooks` を実行し、Moshpit の 4 つのエントリを信頼してください。

**4 · Mosh、tmux、herdr を入れる（任意）。** Mosh はネットワークが変わってもセッションを保ち、tmux と herdr はそれぞれのペインをネイティブに表示します。どれもアプリ内にインストールガイドがあります。

## 対応エージェント

| エージェント | ホーム、Dynamic Island、ライブアクティビティでの状態 | 「あなた待ち」のプッシュ通知 | 「完了」のプッシュ通知 |
| :-- | :-: | :-: | :-: |
| Claude Code | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓（権限の要求） | ✓ |
| Gemini CLI | ✓ | ✓（ツールの権限） | ✓ |
| Qwen Code | ✓ | ✓ | ✓ |
| Qoder | ✓ | ✓ | ✓ |
| Factory（`droid`） | ✓ | ✓ | ✓ |
| CodeBuddy | ✓ | ✓ | ✓ |

*ホストを設定* は、ホスト上で見つかったエージェントの分だけフックをインストールします。tmux では、ホームに出る状態はこのフックから来ます。herdr では herdr 自身が状態を報告するので、設定は何もいりません。プッシュ通知は、エージェントがどこで動いていても（tmux でも、herdr でも、素のシェルでも）フックから送られます。

「あなた待ち」のプッシュ通知は 30 秒待ってから送られるので、机の前で答えた質問がスマホに届くことはありません。「完了」のプッシュ通知にはそのターンにかかった時間が入り、ホストに `jq` があれば、どのプロンプトへの応答だったかも入ります。3 分以上かかったターンは音が鳴り、それより短いものは音なしで届きます。

## 全体の仕組み

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/architecture-dark.svg">
  <img src="docs/assets/architecture-light.svg" width="900" alt="Moshpit の全体像：iPhone と iPad のアプリ（Rust のコアの上に薄い Swift の層）は SSH か Mosh でホストに接続し、tmux や herdr のペインと各エージェントの状態を読み取ります。ホスト上では、エージェントのフックが各ペインに状態を書き込み、「あなた待ち」と「完了」を暗号化用のスクリプトに渡します。暗号化されたプッシュ通知は、暗号文しか見えないリレーと APNs を通ってスマホの通知サービスに届き、そこで復号されます。">
</picture>

アプリは SSH か Mosh でホストに接続し、tmux はコントロールモードで、herdr は herdr 独自のプロトコルで操作します。そのためペインはネイティブに描画され、各エージェントの状態も出力から推測するのではなく、データとして届きます。ホスト上では、各エージェントのフックが小さなスクリプトを呼び出します。このスクリプトはエージェントの状態をペインに記録し、「あなた待ち」と「完了」のときは *ホストを設定* でペアリングした鍵でプッシュ通知を暗号化します。リレーは暗号化されたエンベロープを APNs に転送するだけで、見えるのは暗号文だけです。復号するのはスマホ上の通知サービスです。プロトコルの全体は [docs/PUSH.md](docs/PUSH.md)（英語）にあります。

## できること

### まずはターミナルとして

<table>
<tr>
<td width="50%"><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ja/05-overview.png" alt="tmuxもherdrも、ペインをまとめて見渡せる"></td>
<td width="50%"><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ja/08-keyboard.png" alt="ターミナルのための キーボード"></td>
</tr>
</table>

| | |
| :-- | :-- |
| **SSH と Mosh** | Mosh は予測エコーつき。Wi-Fi と 5G の切り替え、スリープ、ホテルの不安定なネットワークでもセッションは切れません。アプリがバックグラウンドにいる間も生きていた接続は、再接続せずにそのまま使い続けられます |
| **つながらない理由を伝える** | ポートが接続を拒否したのか、名前を解決できなかったのか（またはローカルネットワークの許可がオフなのか）、ホストが応答しなかったのか（VPN を確認してください）、このネットワークからの経路がないのかを示します |
| **tmux コントロールモード** | ネイティブ描画で、ウィンドウとペインの作成、名前の変更、終了ができます。小さな偽物のデスクトップをスマホに押し込むようなことはしません |
| **[herdr](https://github.com/herdrdev/herdr)** | コーディングエージェントのために作られたマルチプレクサ。ペインを 1 つずつ、画面幅いっぱいに表示します |
| **ペインの一覧** | セッションのすべてのウィンドウとペインを一度に並べ、それぞれの画面をライブで映します。tmux でも herdr でも同じです。横にスワイプするとペインを移動できます |
| **tmux が消えたとき** | tmux サーバーが kill されたりクラッシュしたりすると、ホームはそのセッションやエージェントを動いているものとして表示しなくなり、ターミナルには「新規セッション」が出ます |
| **Metal で描画** | wezterm のターミナルコアのフォークをベースに、インライン画像（kitty、iTerm2、sixel）、スクロールバックの検索、ハンドルつきの選択、慣性の効いた履歴スクロールに対応します |
| **入力** | esc / ctrl / alt / 矢印キーのショートカットバー、IME の変換入力、タップできるリンク、カスタムテーマ、7 種類のターミナルフォント |
| **ホスト** | 各ホストには、ホスト鍵から描いた顔（同じ鍵なら必ず同じ顔）か、自分で選んだ絵文字がつきます。バックアップから新しい iPhone に復元すると、古い iPhone に残ったままのパスワードとキーを Moshpit が一覧にするので（キーチェーンの項目は引き継がれません）、入力し直すかインポートし直せます。信頼済みのホスト鍵は信頼済みのまま残ります |

### エージェントの作業のために

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ja/04-island.png" width="250" align="right" alt="アプリを開かなくても、進み具合がわかる">

| | |
| :-- | :-- |
| **「あなた待ち」が先頭** | ホームには、すべてのホストであなたを待っているエージェントが並びます。その下に、作業中のエージェントとその経過時間、待機中のエージェント |
| **ロック画面にプッシュ通知** | エンドツーエンドで暗号化され、アプリを閉じていても届きます。タップすれば、質問してきたペインに入れます |
| **ライブアクティビティ** | ロック画面と Dynamic Island に、誰が待っていて、誰が作業中で、それぞれどれだけ経ったかを表示します |
| **新しいワークツリー** | スマホから、新しい git ワークツリーでエージェントを起動できます。リポジトリを選び、ブランチ名をつけ、必要なら最初のプロンプトも渡せます |
| **ペインに共有** | どのアプリからでも、画像をエージェントのペインに直接共有できます |

<br clear="right">

### エージェントが挙げたファイル

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ja/06-preview.png" width="250" align="right" alt="エージェントが変えたファイルを、タップで確認">

エージェントの出力にあるパスをタップすると、そのファイルがスマホで開きます。画像はプレビューされ、動画は再生され、テキストはテキストとして読めます。そこからファイルのあるフォルダを見て回ったり、ファイルを共有したりもできます。

<br clear="right">

### 音声入力

音声入力はデバイス上で処理します。**Qwen3-ASR**（0.6B、ダウンロードは初回の 1 回だけ）は、英語の用語が混じった中国語の発話をひとつの文として認識し、ペインの画面に出ているファイル名、ブランチ、エージェントをホットワードとして使います。**Whisper** と Apple のオンデバイスエンジンも使えます。音声がスマホの外に出ることはありません。

### iPad

<img src="https://blog.cluas.eu.org/moshpit/shots/ipad/ja/02-home-terminal.png" width="360" align="right" alt="ホームとターミナルを並べて">

左にホスト、右にターミナル。ハードウェアキーボードでは、<kbd>⌘K</kbd> ですべてのペインを表示、<kbd>⌘1</kbd>–<kbd>⌘9</kbd> でウィンドウを選択、<kbd>⌘W</kbd> でホームに戻ります。

<br clear="right">

### コンパニオンとアイコン

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ja/07-pet.png" width="250" align="right" alt="待ち時間の相棒を、たまごから育てる">

| | |
| :-- | :-- |
| **ピクセルのコンパニオン** | ホームでたまごから孵り、いちばんあなたを必要としているエージェントのカードにとまって、そのエージェントの状態に合わせて姿を変えます。Dynamic Island とライブアクティビティでもそばにいます。オンにすると、各ホストの顔も、同じ鍵から描かれたピクセルの生き物になります |
| **22 種類のアイコン** | すべて設定にあります。デフォルトはピクセルのたまご。ほかに、6 種類のコンパニオンがそれぞれペインの向こうから顔を出すアイコン（どれにもキラキラ版があります）、Classic、さらに 8 つ。コンパニオンが孵ると、その種類のアイコンに切り替えるかどうかをホームで尋ねます |

<br clear="right">

ここにあるスクリーンショットはすべて App Store 用のもので、Screenshots ワークフローで作られ、ワークフローを最後まで実行するたびに更新されます。すべての言語と iPad 版は[スクリーンショット一覧](https://blog.cluas.eu.org/moshpit/shots/#ja)で見られます。

## 隠しごとのない作り

- 買い切りです。サブスクリプションも、アカウントも、機能ごとのプランもありません。
- 何も収集しません。アナリティクスもトラッキングもありません。鍵は iOS のキーチェーンに保存され、通信先はあなた自身のサーバーと、暗号文しか見えないプッシュリレーだけです。
- Moshpit は、確認せずにホストへ何かをインストールすることも、勝手にセッションを作ることもありません。接続の品質が落ちたときは、黙って接続方式を切り替えたりせず、そのことを伝えます。

## 内部の構成

Moshpit の大部分は Rust です。コアは `crates/` にあります。SSH のトランスポート（`ssh`、russh ベース）、Mosh のプロトコル（`mosh`）、接続、再接続、再開を受け持つセッション層（`session`）、tmux コントロールモード（`tmux`）と herdr（`herdr`）、ターミナルエミュレータ（`term`、中身は `vendor/wezterm` にある wezterm のフォーク）、アプリが保存するデータ（`store`）、エージェントの状態機械（`agents`）、コンパニオン（`pet`）、そしてホストの設定とプッシュ通知のペアリング（`hostcare`、`install`、`seal`）です。画面も Rust で、[NSH](https://nsh.dev) を使って `ui/` に書かれており、画面のモデルは `crates/app` にあります。

iOS では、薄い Swift の層がひとつの静的ライブラリ（`crates/ffi`）を通してこれらすべてを載せています。Swift 側が受け持つのは、アプリのシェル、ターミナルビューの Metal 描画とテキスト入力、キーチェーン、音声入力（sherpa-onnx の onnxruntime で動く Qwen3-ASR、WhisperKit で動く Whisper、Apple の音声認識エンジン）、そして iOS が自前の仕組みでしか作らせない部分（ライブアクティビティとウィジェット、`crates/seal-ffi` を通してプッシュ通知を復号する通知サービス、共有エクステンション）です。Android アプリも同じコアと画面を使っています（`crates/android`、`android/`）。こちらは開発中で、まだリリースしていません。

<details>
<summary><b>リポジトリの構成</b></summary>

<br>

| パス | 中身 |
| :-- | :-- |
| `crates/` | Rust のコア。関心ごとに 1 クレート：`ssh`、`mosh`、`session`、`tmux`、`herdr`、`term`、`stream`（コアとターミナルビューの間でペインのバイト列を運ぶ）、`store`、`agents`、`pet`（コンパニオン）、`seal`（プッシュ通知のエンベロープ）、`seal-ffi`（通知サービスが `seal` を呼ぶ唯一の入り口）、`hostcare`、`install`、`app`（画面のモデル）、`runtime`、`copy`（中国語の文言が守るルール）、`ffi`（iOS がリンクする静的ライブラリ）、`android`（Android アプリの Rust 側） |
| `ui/` | Rust と NSH で書かれた画面。iOS と Android で共有 |
| `vendor/wezterm/` | wezterm のターミナルエミュレータのフォークで、`crates/term` の裏で動きます。変更点の一覧は [docs/WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md) |
| `Moshpit/` | iOS 側の土台：アプリのシェル、各画面の Swift 側、ターミナルビューの Metal 描画とテキスト入力、キーチェーン、音声入力（Qwen3-ASR、Whisper、Apple のエンジン） |
| `Extensions/` | 3 つのアプリ拡張：`MoshpitIsland/`（ライブアクティビティとロック画面のウィジェット、Vibe Island）、`MoshpitPush/`（プッシュ通知をデバイス上で復号する通知サービス）、`MoshpitShare/`（画像をエージェントのペインに渡す） |
| `Packages/MoshpitKit/` | アプリと 3 つの拡張すべてがリンクするローカル Swift パッケージ：ログ、ライブアクティビティのペイロード、暗号化したプッシュ通知のエンベロープとペアリングの保存 |
| `android/` | Android アプリ（Gradle）。開発中 |
| `Tests/` | `MoshpitTests/` はユニットテスト、`MoshpitUITests/` は UI テスト |
| `push-relay/` | ホストと APNs の間に立つ、状態を持たない Go 製のリレー。デプロイ用のマニフェストは `deploy/` の下 |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md)、[PUSH.md](docs/PUSH.md)（プッシュ通知のプロトコルの全体）、[WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md)、`nsh/`（Rust のコアと NSH の画面についての設計メモ）、その他の設計メモ |
| `scripts/` | `rust/` iOS ライブラリのビルドと Swift バインディング、`host/` 開発用ホストにインストールされるシェルスクリプト、`gen/` ジェネレータ、`verify/` エンドツーエンドのチェック、`spikes/` 実験、`wezterm/` フォークへの上流の取り込み、`release/` アーカイブ用ツール |

</details>

アプリはデフォルトでメンテナーが運用するリレーを使います。リレーに何が見えて何が見えないか、自分でリレーを立てる方法は [docs/PUSH.md](docs/PUSH.md) にあります。

## ソースからビルド

必要なのは、Xcode 27 の入った macOS（アプリのデプロイメントターゲットは iOS 18）、`brew install xcodegen` で入れる [XcodeGen](https://github.com/yonaskolb/XcodeGen)、[rustup](https://rustup.rs) で入れる Rust です。リポジトリは `rust-toolchain.toml` でツールチェーンを固定しているので、初回に rustup が正しいものを入れてくれます。

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # 正本は project.yml。.xcodeproj はそこから生成されます
open Moshpit.xcodeproj
```

**Moshpit** スキームとシミュレータを選んで ⌘R を押します。アプリターゲットの「Build Rust」フェーズが `scripts/rust/build-ios.sh` を実行し、`crates/ffi` をシミュレータ用または実機用の `libmoshpit_ffi.a` にビルドします。必要な Rust ターゲット（`aarch64-apple-ios-sim`、`aarch64-apple-ios`）は初回に追加されます。Debug ビルドは cargo の `app-test` プロファイル、Release ビルドは `app` を使います。初回のビルドは Rust の依存グラフ全体をコンパイルするので時間がかかりますが、2 回目以降は差分ビルドです。Swift パッケージも初回のビルドで解決されます。

実機ビルドには、`Signing.xcconfig` に自分の Team ID を書く必要があります。手順はそのファイル内のコメントと [docs/install-free-account.md](docs/install-free-account.md) にあり、無料の Apple ID を使う方法も載っています。

### テスト

Rust 側は、CI と同じように次のコマンドで実行します。

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p moshpit-term        # クレートを 1 つだけテスト
```

iOS アプリは、CI が Xcode 27 のイメージで実行するのと同じ手順です。

```sh
scripts/rust/test-build-ios.sh    # ビルドはせず、Rust フェーズのプロファイルとリンクだけを確認
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' CODE_SIGNING_ALLOWED=NO
```

いくつかのテストスイートは、署名なしのビルドでは（App Group のコンテナが必要なため）、また iPad シミュレータ以外では（ハードウェアキーボードのショートカット）スキップされます。一部の UI フローも、今の画面に合わせて書き直すまではスキップされます。どれがスキップされたかはログに出ます。

Android アプリのビルドとテストは `android/ci.sh` で行います。JDK 17 があれば、必要なもの（SDK の各コンポーネント、NDK、Gradle）はスクリプトが自分で取得します。

## コントリビュート

作業の流れとこのリポジトリの書き方の決まりは [CONTRIBUTING.md](CONTRIBUTING.md) に、セキュリティに関わる報告の前に読むべきことは [SECURITY.md](SECURITY.md) にあります（どちらも英語）。参加する人は全員、[行動規範](CODE_OF_CONDUCT.md)に従います。

Moshpit は、中国語の開発者コミュニティ [LINUX DO](https://linux.do) でも紹介しています。質問やアイデアは、Issue トラッカーと同じようにそちらでも歓迎します。

## ライセンス

Moshpit はフリーソフトウェアで、[GNU 一般公衆利用許諾書 v3.0](LICENSE)（GPL-3.0-only）のもとで公開しています。このライセンスの条件に従って、ビルド、改変、再配布ができます。App Store 版は、著作権を持つメンテナーが公開しています。サードパーティのコンポーネントにはそれぞれのライセンスが適用され、[NOTICES.md](NOTICES.md) に一覧があります。
