#!/usr/bin/env python3
"""Generate Offhook/Resources/Localizable.xcstrings and OffhookIsland/Localizable.xcstrings.

Source language: en (the key IS the English value).
Each entry carries complete zh-Hans + ja translations.
SAME = identical value in every language (brand names, protocol words, glyphs).
"""
import json, os

SAME = object()

# key -> (zh-Hans, ja) | SAME | {"plural": {"en_one":..., "en_other":..., "zh":..., "ja":...}}
S = {}

def add(key, zh=SAME, ja=None):
    S[key] = SAME if zh is SAME else (zh, ja)

def plural(key, en_one, zh, ja):
    S[key] = {"plural": True, "en_one": en_one, "zh": zh, "ja": ja}

# ---------- Brand / protocol / glyphs (identical everywhere) ----------
for k in ["Offhook", "Vibe Island", "MOSH", "SRTT", "REC",
          "ED25519", "ECDSA-sk", "RSA-4096", "tmux", "paste", "tmux prefix",
          "⌘B", "＋", "22", "60000", "61000", "/usr/local/bin/mosh-server",
          "ABCdef 012 ~/ssh $", "%lldms", "%lldpt", "Wi-Fi → 5G",
          # Glyphs + technical formats the compiler extracts from Text literals;
          # identical in every language by design.
          "%@@%@", "%lld:%@", "+", "−", "/", "·", "—", "›", "→",
          "└─", "▸", "▾", "✓", "➜"]:
    add(k, SAME)

# ---------- Common actions ----------
add("Done", "完成", "完了")
add("Cancel", "取消", "キャンセル")
add("Save", "保存", "保存")
add("OK", "好", "OK")
add("Delete", "删除", "削除")
add("Edit", "编辑", "編集")
add("Open", "打开", "開く")
add("Connect", "连接", "接続")
add("Disconnect", "断开连接", "切断")
add("Settings", "设置", "設定")
add("Continue", "继续", "続ける")

# ---------- Home (AttachHomeView) ----------
add("Your servers, on call.", "你的服务器，随叫随到。", "あなたのサーバー、いつでもオンコール。")
add("No connections yet", "还没有连接", "接続はまだありません")
add("Tap ＋ to add your first server. Offhook keeps your sessions alive across Wi-Fi / 5G handoff.",
    "点按 ＋ 添加第一台服务器。Offhook 会在 Wi-Fi / 5G 切换时保持会话不断线。",
    "＋をタップして最初のサーバーを追加。Offhook は Wi-Fi / 5G の切り替えをまたいでセッションを維持します。")
add("SIGNAL FOR YOUR AGENTS", "智能体信号站", "エージェントのシグナル")
add("Delete %@?", "删除 %@？", "%@を削除しますか？")
add("connection", "连接", "接続")
add("Removes the saved server and its stored credentials from this device.",
    "从本设备移除已保存的服务器及其存储的凭据。",
    "保存済みのサーバーと、このデバイスに保存された認証情報を削除します。")
add("Delete Connection", "删除连接", "接続を削除")
add("SESSIONS", "会话", "セッション")
add("%lldh roamed", "漫游 %lld 小时", "ローミング %lld 時間")
add("%lldh up", "在线 %lld 小时", "稼働 %lld 時間")
plural("%lld windows", "%lld window", "%lld 个窗口", "%lld個のウィンドウ")
plural("%lld panes", "%lld pane", "%lld 个窗格", "%lld個のペイン")
plural("%lld keys", "%lld key", "%lld 个密钥", "%lld個のキー")

# ---------- Add Connection ----------
add("CONNECTION", "连接", "接続")
add("Name", "名称", "名前")
add("Host", "主机", "ホスト")
add("Port", "端口", "ポート")
add("Username", "用户名", "ユーザー名")
add("AUTHENTICATION", "认证", "認証")
add("Password", "密码", "パスワード")
add("SSH Key", "SSH 密钥", "SSH キー")
add("Key", "密钥", "キー")
add("Paste PEM", "粘贴 PEM", "PEM を貼り付け")
add("Paste a PEM instead…", "改为粘贴 PEM…", "代わりに PEM を貼り付け…")
add("Private Key (PEM)", "私钥（PEM）", "秘密鍵（PEM）")
add("ROAMING · MOSH", "漫游 · MOSH", "ローミング · MOSH")
add("Mosh keeps the session alive across Wi-Fi / 5G handoff and sleep/wake. UDP must be open server-side; the client picks an unused port within range. With tmux, Offhook attaches to your existing sessions and never creates or restyles them; only sessions you create through Offhook get its native look (status bar hidden, restored on disconnect).",
    "Mosh 让会话在 Wi-Fi / 5G 切换及休眠唤醒后保持存活。服务器端需开放 UDP；客户端会在范围内挑选一个未占用的端口。使用 tmux 时，Offhook 只附加到你已有的会话，绝不创建或改样式；只有通过 Offhook 创建的会话才会应用其原生外观（隐藏状态栏，断开时恢复）。",
    "Mosh は Wi-Fi / 5G の切り替えやスリープ/復帰をまたいでセッションを維持します。サーバー側で UDP を開放してください。クライアントは範囲内の未使用ポートを選びます。tmux では Offhook は既存セッションにアタッチするだけで、作成やスタイル変更は行いません。Offhook から作成したセッションのみネイティブな外観（ステータスバー非表示、切断時に復元）になります。")
add("Use Mosh", "使用 Mosh", "Mosh を使用")
add("Wrap SSH with mosh-server (UDP)", "连接时用 mosh-server (UDP) 包装 SSH", "SSH を mosh-server (UDP) でラップ")
add("UDP Port Range", "UDP 端口范围", "UDP ポート範囲")
add("Predict Mode", "预测模式", "予測モード")
add("Roam on Cellular", "蜂窝网络漫游", "モバイル通信でローミング")
add("Reconnect over 5G/LTE when Wi-Fi drops", "Wi-Fi 断开时通过 5G/LTE 重连", "Wi-Fi が切れたら 5G/LTE で再接続")
add("ADVANCED", "高级", "詳細設定")
add("Custom tmux Path", "自定义 tmux 路径", "カスタム tmux パス")
add("Compress Output", "压缩输出", "出力を圧縮")
add("Add Connection", "添加连接", "接続を追加")
add("Edit Connection", "编辑连接", "接続を編集")
add("Could not save", "无法保存", "保存できませんでした")

# ---------- Settings ----------
add("DISPLAY", "显示", "表示")
add("Font", "字体", "フォント")
add("Font Size", "字号", "フォントサイズ")
add("Theme", "主题", "テーマ")
add("CURSOR", "光标", "カーソル")
add("Shape and color apply to all SSH / mosh sessions. With trail on, characters that mosh's predictive echo shows ahead of the server are marked with a translucent trail until confirmed.",
    "形状与颜色对所有 SSH / mosh 会话生效。开启 trail 时，mosh 预测回显领先服务端的字符会用半透明拖尾标出未被确认的范围。",
    "形状と色はすべての SSH / mosh セッションに適用されます。trail をオンにすると、mosh の予測エコーがサーバーより先に表示した文字は、確定するまで半透明のトレイルで示されます。")
add("Shape", "形状", "形状")
add("Applies to all sessions", "形状对所有会话生效", "すべてのセッションに適用")
add("Block", "方块", "ブロック")
add("Bar", "竖线", "バー")
add("Underline", "下划线", "下線")
add("Color", "颜色", "カラー")
add("Current session · switches to amber while roaming", "当前会话 · 漫游态自动切换为 amber", "現在のセッション · ローミング中は amber に切り替え")
add("Blink", "闪烁", "点滅")
add("1.1s cadence, follows the iOS system default", "1.1s 节奏，遵循 iOS 系统默认", "1.1 秒周期、iOS のシステムデフォルトに準拠")
add("Trail on predict", "预测拖尾", "予測時のトレイル")
add("Leaves a teal trail behind the cursor while mosh predicts ahead of the server",
    "mosh 预测领先服务端时光标后留青色拖尾",
    "mosh がサーバーより先行して予測する間、カーソルの後ろにティールのトレイルを残します")
add("BEHAVIOR", "行为", "動作")
add("Keep Connections Alive", "保持连接存活", "接続を維持")
add("Send keepalive pings so the server does not drop idle connections",
    "发送保活探测，防止服务器断开空闲连接",
    "キープアライブを送信し、アイドル接続がサーバーに切断されないようにします")
add("MOSH · ROAMING", "MOSH · 漫游", "MOSH · ローミング")
add("Mosh runs over UDP and survives IP changes. If your server is behind a strict firewall, open the port range above outbound from your iPhone.",
    "Mosh 基于 UDP，可在 IP 变化后存活。若服务器位于严格防火墙之后，请从 iPhone 出方向开放上面的端口范围。",
    "Mosh は UDP 上で動作し、IP の変化にも耐えます。サーバーが厳格なファイアウォール内にある場合は、iPhone からの送信方向で上記のポート範囲を開放してください。")
add("Mosh by default", "默认使用 Mosh", "デフォルトで Mosh を使用")
add("Wrap new SSH hosts with mosh-server on connect", "新建 SSH 主机连接时自动用 mosh-server 包装", "新しい SSH ホストを接続時に mosh-server でラップ")
add("Predictive Echo", "预测回显", "予測エコー")
add("Show typed characters locally before server confirms", "在服务器确认前先在本地显示输入的字符", "サーバー確認前に入力文字をローカルで表示")
add("Server binary", "服务器程序", "サーバーバイナリ")
add("UDP port range", "UDP 端口范围", "UDP ポート範囲")
add("KEYBOARD · KEYS", "键盘 · 密钥", "キーボード · キー")
add("Shortcuts", "快捷键", "ショートカット")
add("SSH Keys", "SSH 密钥", "SSH キー")
add("NOTIFICATIONS", "通知", "通知")
add("Offhook watches the active tmux session for the terminal bell and posts a local alert when your agent needs attention.",
    "Offhook 会监听活动 tmux 会话的终端响铃，并在你的智能体需要关注时发出本地提醒。",
    "Offhook はアクティブな tmux セッションのターミナルベルを監視し、エージェントが注意を必要とするとローカル通知を出します。")
add("Notifications", "通知", "通知")
add("Alert when the agent rings the bell", "智能体响铃时提醒", "エージェントがベルを鳴らしたら通知")
add("Live Activity", "实时活动", "ライブアクティビティ")
add("Show agent session status in the Dynamic Island", "在灵动岛显示智能体会话状态", "Dynamic Island にエージェントの状態を表示")
add("How notifications work", "通知的工作原理", "通知のしくみ")
add("VOICE INPUT", "语音输入", "音声入力")
add("Dictate commands instead of typing. Planned for a future update.",
    "用语音口述命令，无需打字。计划在后续版本提供。",
    "タイプの代わりに音声でコマンドを入力。今後のアップデートで提供予定です。")
add("Enable Voice Input", "启用语音输入", "音声入力を有効化")
add("Coming soon", "即将推出", "近日公開")
# Notification info sheet
add("Bell = attention", "响铃 = 需要关注", "ベル = 要注意")
add("When a tmux pane rings the terminal bell (BEL) — which Claude Code and most CLIs emit when they finish or need input — Offhook posts a local notification and flips the Vibe Island to “needs attention.”",
    "当 tmux 窗格触发终端响铃（BEL）——Claude Code 和多数 CLI 在完成或需要输入时都会发出——Offhook 会发送本地通知，并把 Vibe Island 切换为“需要关注”。",
    "tmux ペインがターミナルベル（BEL）を鳴らすと——Claude Code やほとんどの CLI は完了時や入力待ちで発します——Offhook はローカル通知を送り、Vibe Island を「要注意」に切り替えます。")
add("While a session is attached, the Dynamic Island shows whether the agent is working, idle, or waiting on you. Tapping it deep-links straight back to that pane.",
    "会话附加期间，灵动岛会显示智能体正在工作、空闲，还是在等你。点按即可直接深链回到该窗格。",
    "セッションのアタッチ中、Dynamic Island はエージェントが作業中・アイドル・あなた待ちのいずれかを表示します。タップするとそのペインへ直接ジャンプします。")
add("Local, not push", "本地通知，而非推送", "ローカル通知（プッシュではありません）")
add("Alerts are generated on-device from the live session — there’s no cloud push server. They fire while Offhook is in the foreground or recently backgrounded; a fully suspended app won’t poll. iOS will ask for notification permission the first time you connect with Notifications on.",
    "提醒由设备端的实时会话生成——没有云端推送服务器。它们在 Offhook 处于前台或刚进入后台时触发；完全挂起的应用不会轮询。首次在开启通知的情况下连接时，iOS 会请求通知权限。",
    "通知はライブセッションからデバイス上で生成されます——クラウドのプッシュサーバーはありません。Offhook がフォアグラウンドまたは直近バックグラウンドの間に発火し、完全にサスペンドされたアプリはポーリングしません。通知をオンにして初めて接続する際、iOS が通知の許可を求めます。")
# Server binary editor
add("MOSH SERVER PATH", "MOSH SERVER 路径", "MOSH SERVER パス")
add("The mosh-server executable on the remote host. Override if it isn't on PATH (e.g. /opt/homebrew/bin/mosh-server).",
    "远程主机上的 mosh-server 可执行文件。若不在 PATH 中可在此覆盖（如 /opt/homebrew/bin/mosh-server）。",
    "リモートホスト上の mosh-server 実行ファイル。PATH にない場合はここで上書きします（例: /opt/homebrew/bin/mosh-server）。")
add("Server Binary", "服务器程序", "サーバーバイナリ")
# UDP editor
add("UDP PORT RANGE", "UDP 端口范围", "UDP ポート範囲")
add("mosh binds one UDP port in this range per session. Open it outbound from your iPhone and inbound on the server (default 60000–61000).",
    "mosh 每个会话会在此范围内绑定一个 UDP 端口。请在 iPhone 出方向和服务器入方向开放（默认 60000–61000）。",
    "mosh はセッションごとにこの範囲の UDP ポートを 1 つバインドします。iPhone 側は送信方向、サーバー側は受信方向で開放してください（デフォルト 60000–61000）。")
add("From", "起始", "開始")
add("To", "结束", "終了")
add("Invalid range", "范围无效", "範囲が無効です")
add("Enter numeric ports.", "请输入数字端口。", "数値のポートを入力してください。")
add("Ports must be 1–65535.", "端口必须在 1–65535 之间。", "ポートは 1–65535 の範囲で指定してください。")
add("From must be ≤ To.", "起始值必须 ≤ 结束值。", "開始は終了以下にしてください。")

# ---------- SSH Keys ----------
add("THIS DEVICE · SECURE ENCLAVE", "本机 · SECURE ENCLAVE", "このデバイス · SECURE ENCLAVE")
add("Hardware-backed keys cannot be exported; every signature triggers Face ID.",
    "硬件支持的密钥不可导出，每次签名会触发 Face ID。",
    "ハードウェア保護されたキーは書き出せません。署名のたびに Face ID が起動します。")
add("No device key yet — generate one with ＋", "还没有设备密钥 — 点按 ＋ 生成", "デバイスキーはまだありません — ＋で生成")
add("IMPORTED", "已导入", "インポート済み")
add("No imported keys", "没有已导入的密钥", "インポートされたキーはありません")
add("Delete Key", "删除密钥", "キーを削除")
add("unused since %@", "自 %@ 起未使用", "%@ 以降未使用")
# Add Key
add("Add Key", "添加密钥", "キーを追加")
add("METHOD", "方式", "方式")
add("Generates the key pair on device. The private key can optionally be held in the Secure Enclave — used only for signing, never leaving the chip.",
    "本地生成密钥对。私钥可选 Secure Enclave 托管，仅签名时调用，不出芯片。",
    "デバイス上で鍵ペアを生成します。秘密鍵は Secure Enclave に保管でき、署名時のみ使用され、チップの外には出ません。")
add("Generate", "生成", "生成")
add("Import", "导入", "インポート")
add("Hardware", "硬件", "ハードウェア")
add("HARDWARE KEY", "硬件密钥", "ハードウェアキー")
add("ECDSA-sk keys live on hardware tokens such as a YubiKey. iOS cannot enumerate them directly yet; paste the public key line (sk-ecdsa-sha2-nistp256@openssh.com …) under Import.",
    "ECDSA-sk 密钥保存在 YubiKey 等硬件令牌上。iOS 暂不支持直接枚举；请在 Import 里粘贴它的公钥行（sk-ecdsa-sha2-nistp256@openssh.com …）。",
    "ECDSA-sk キーは YubiKey などのハードウェアトークンに保存されます。iOS はまだ直接列挙できません。Import で公開鍵行（sk-ecdsa-sha2-nistp256@openssh.com …）を貼り付けてください。")
add("KEY DETAILS", "密钥详情", "キーの詳細")
add("ED25519 recommended: 32-byte keys, fast signatures, no practical break to date.",
    "推荐 ED25519：32 字节密钥，签名快，目前未被实战攻破。",
    "ED25519 推奨：32 バイト鍵で署名が高速、現時点で実用的な攻撃は知られていません。")
add("Algorithm", "算法", "アルゴリズム")
add("Comment", "备注", "コメント")
add("PROTECTION", "保护", "保護")
add("Secure Enclave keys are limited to ECDSA-P256; the algorithm adjusts automatically when SE is enabled.",
    "Secure Enclave 密钥的算法限制为 ECDSA-P256；切换到 SE 时算法将自动调整。",
    "Secure Enclave のキーは ECDSA-P256 に限定されます。SE を有効にするとアルゴリズムは自動調整されます。")
add("Passphrase", "口令", "パスフレーズ")
add("Confirm Passphrase", "确认口令", "パスフレーズを確認")
add("Require Face ID", "需要 Face ID", "Face ID を要求")
add("Face ID confirmation before every signature", "每次签名前需 Face ID 确认", "署名のたびに Face ID で確認")
add("Store in Secure Enclave", "存入 Secure Enclave", "Secure Enclave に保存")
add("Hardware-backed · private key cannot be exported", "硬件托管 · 私钥不可导出", "ハードウェア保護 · 秘密鍵は書き出し不可")
add("PRIVATE KEY", "私钥", "秘密鍵")
add("Paste an OpenSSH / PEM private key; the public key line is used to compute the fingerprint (optional).",
    "粘贴 OpenSSH / PEM 私钥；公钥行用于计算指纹（可选）。",
    "OpenSSH / PEM の秘密鍵を貼り付けます。公開鍵行はフィンガープリントの計算に使われます（任意）。")
add("Public key line (optional)", "公钥行（可选）", "公開鍵行（任意）")
add("BIND TO HOSTS", "绑定主机", "ホストにバインド")
add("(optional)", "（可选）", "（任意）")
add("When bound, this key is used only for the selected hosts by default; change it anytime in the SSH Keys detail page.",
    "绑定后此密钥默认仅用于所选主机；可在 SSH Keys 详情页随时改动。",
    "バインドすると、このキーはデフォルトで選択したホストにのみ使われます。SSH キーの詳細ページでいつでも変更できます。")
add("PREVIEW · SHA256", "预览 · SHA256", "プレビュー · SHA256")
add("—:—:—:—:—:—:—:—:—:—  (shown after generation)", "—:—:—:—:—:—:—:—:—:—  (生成后显示)", "—:—:—:—:—:—:—:—:—:—  （生成後に表示）")
add("Key error", "密钥错误", "キーエラー")
add("WEAK", "弱", "弱い")
add("FAIR", "中", "普通")
add("STRONG", "强", "強い")

# ---------- Shortcuts ----------
add("PREVIEW", "预览", "プレビュー")
add("Drag ≡ to reorder; the red − removes it from the toolbar. 12 items max.",
    "拖动 ≡ 排序，红色 − 移出工具条。最多 12 项。",
    "≡ をドラッグして並べ替え、赤い − でツールバーから外します。最大 12 項目。")
add("IN TOOLBAR · %lld/%lld", "工具条内 · %lld/%lld", "ツールバー内 · %lld/%lld")
add("CUSTOM", "自定义", "カスタム")
add("Tap ＋ in the top right to create a custom shortcut — key combos, text snippets, and command chains.",
    "点击右上 ＋ 新建自定义快捷键 — 支持按键组合、文本片段、命令链。",
    "右上の＋でカスタムショートカットを作成 — キーコンボ、テキストスニペット、コマンドチェーンに対応。")
add("No custom shortcuts yet", "还没有自定义快捷键", "カスタムショートカットはまだありません")
add("AVAILABLE", "可用", "利用可能")
add("Add Shortcut", "添加快捷键", "ショートカットを追加")
add("Edit Shortcut", "编辑快捷键", "ショートカットを編集")
add("Live preview of your input below. Saved shortcuts land in the Custom group of the Shortcuts toolbar.",
    "实时映射你在下方的输入。保存后会自动落到 Shortcuts 工具条的 Custom 分组。",
    "下の入力をリアルタイムでプレビューします。保存するとショートカットツールバーのカスタムグループに入ります。")
add("TYPE", "类型", "種類")
add("Key Combo: sends modifiers + key. Text: types a string as-is. Command: submits one command with Return.",
    "Key Combo：发送修饰键 + 键。Text：直接键入字符串。Command：按 Return 提交一条命令。",
    "Key Combo：修飾キー＋キーを送信。Text：文字列をそのまま入力。Command：Return でコマンドを 1 つ送信。")
add("Key Combo", "按键组合", "キーコンボ")
add("Text", "文本", "テキスト")
add("Command", "命令", "コマンド")
add("TRIGGER", "触发", "トリガー")
add("Tap modifiers to toggle them and enter a single character as the main key; the mobile keyboard triggers the raw scancode automatically.",
    "点击修饰键开关，单字符键输入主键；移动键盘会自动触发原始 scancode。",
    "修飾キーをタップして切り替え、メインキーは 1 文字で入力します。モバイルキーボードは生のスキャンコードを自動送信します。")
add("Modifiers", "修饰键", "修飾キー")
add("Main key", "主键", "メインキー")
add("ACTION", "动作", "アクション")
add("In Key Combo mode the payload is the escape sequence sent to the PTY, honoring the mosh / tmux transcription rules.",
    "Key Combo 模式下 Payload 是发往 PTY 的转义序列，可读 mosh / tmux 转写规则。",
    "Key Combo モードではペイロードは PTY へ送るエスケープシーケンスで、mosh / tmux の転写規則に従います。")
add("Payload (text or command)", "Payload（文本或命令）", "ペイロード（テキストまたはコマンド）")
add("Append Return", "追加 Return", "Return を追加")
add("Automatically appends ⏎ to submit", "按下后自动追加 ⏎ 提交", "送信時に自動で ⏎ を追加")
add("Repeat on hold", "长按重复", "長押しでリピート")
add("Repeats at 30/s after a 0.4s hold", "长按 0.4s 后以 30/s 速率重复", "0.4 秒の長押し後、毎秒 30 回リピート")
add("Chip Label is capped at 6 characters; Description appears in the edit list to keep shortcuts recognizable.",
    "Chip Label 最多 6 字符；Description 会出现在编辑列表里方便辨认。",
    "チップラベルは最大 6 文字。説明は編集リストに表示され、見分けやすくなります。")
add("Chip Label", "标签", "チップラベル")
add("Description", "描述", "説明")
add("SCOPE", "作用域", "スコープ")
add("Available everywhere when nothing is selected. When bound, it appears only in the toolbar of the selected hosts, saving slots in the 12-slot budget.",
    "不选时全局可用。绑定后仅在所选主机的会话工具条出现，节省 12 个槽位预算。",
    "未選択ならどこでも使えます。バインドすると選択ホストのツールバーにのみ表示され、12 スロットの枠を節約できます。")
add("Only in tmux", "仅在 tmux 中", "tmux 内のみ")
add("Show only while the current session runs tmux", "仅当当前会话有 tmux 进程时显示", "現在のセッションで tmux 実行中のみ表示")
add("SLOT %lld/%lld", "槽位 %lld/%lld", "スロット %lld/%lld")
add("unnamed", "未命名", "名称未設定")
add("%@ · sends %@", "%1$@ · 发送 %2$@", "%1$@ · %2$@ を送信")
add("%@ · types \"%@\"", "%1$@ · 键入 \"%2$@\"", "%1$@ · \"%2$@\" を入力")
add("%@ · runs %@", "%1$@ · 运行 %2$@", "%1$@ · %2$@ を実行")

# ---------- Terminal ----------
add("Connection Error", "连接错误", "接続エラー")
add("⚠️ Host Key Changed", "⚠️ 主机密钥已变更", "⚠️ ホストキーが変更されました")
add("The key for %@:%lld does NOT match the one stored on this device. This can mean the server was reinstalled — or that the connection is being intercepted.\n\nStored: %@\nNew: %@",
    "%1$@:%2$lld 的密钥与本设备存储的不一致。这可能意味着服务器被重装——也可能是连接正被拦截。\n\n已存储：%3$@\n新密钥：%4$@",
    "%1$@:%2$lld のキーがこのデバイスに保存されたものと一致しません。サーバーが再インストールされたか、接続が傍受されている可能性があります。\n\n保存済み：%3$@\n新規：%4$@")
add("Trust New Key", "信任新密钥", "新しいキーを信頼")
add("New Host", "新主机", "新しいホスト")
add("First connection to %@:%lld.\n\nKey fingerprint:\n%@\n\nVerify it matches the server (e.g. `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`).",
    "首次连接 %1$@:%2$lld。\n\n密钥指纹：\n%3$@\n\n请核对其与服务器一致（如 `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`）。",
    "%1$@:%2$lld への初回接続です。\n\nキーのフィンガープリント：\n%3$@\n\nサーバー側と一致するか確認してください（例: `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub`）。")
add("Trust", "信任", "信頼")
add("Connecting…", "连接中…", "接続中…")
add("roaming…", "漫游中…", "ローミング中…")
add("Predict ON", "预测已开", "予測オン")
add("No tmux sessions", "没有 tmux 会话", "tmux セッションがありません")
add("This server has no running sessions.\nOffhook only attaches — create the first one to start.",
    "该服务器上没有正在运行的会话。\nOffhook 只做附加 — 创建第一个会话以开始。",
    "このサーバーには実行中のセッションがありません。\nOffhook はアタッチのみ行います — 最初のセッションを作成して始めましょう。")
add("Creating…", "创建中…", "作成中…")
add("Create Session", "创建会话", "セッションを作成")
add("Attaching tmux…", "正在附加 tmux…", "tmux にアタッチ中…")
add("Mosh: %@", "Mosh：%@", "Mosh: %@")
# tmux sheets
add("Windows", "窗口", "ウィンドウ")
add("Sessions", "会话", "セッション")
add("Select Pane", "选择窗格", "ペインを選択")
add("Swipe to switch", "滑动切换", "スワイプで切り替え")
add("Tap to focus pane", "点按聚焦窗格", "タップでペインにフォーカス")
add("no panes", "没有窗格", "ペインなし")
add("attached", "已附加", "アタッチ済み")
add("detached", "已分离", "デタッチ済み")

# ---------- Vibe Island (shared with widget) ----------
ISLAND = {}
def island(key, zh, ja):
    ISLAND[key] = (zh, ja)
    S[key] = (zh, ja)
island("working", "工作中", "作業中")
island("needs attention", "需要关注", "要注意")
island("idle", "空闲", "アイドル")
ISLAND["%lldms"] = SAME  # latency unit chip in the Island UI


def unit(value):
    return {"stringUnit": {"state": "translated", "value": value}}


def entry(key, spec):
    locs = {}
    if spec is SAME:
        locs["zh-Hans"] = unit(key)
        locs["ja"] = unit(key)
    elif isinstance(spec, dict) and spec.get("plural"):
        locs["en"] = {"variations": {"plural": {
            "one": {"stringUnit": {"state": "translated", "value": spec["en_one"]}},
            "other": {"stringUnit": {"state": "translated", "value": key}},
        }}}
        locs["zh-Hans"] = unit(spec["zh"])
        locs["ja"] = unit(spec["ja"])
    else:
        zh, ja = spec
        locs["zh-Hans"] = unit(zh)
        locs["ja"] = unit(ja)
    return {"localizations": locs}


def write_catalog(path, table):
    catalog = {
        "sourceLanguage": "en",
        "strings": {k: entry(k, v) for k, v in sorted(table.items())},
        "version": "1.0",
    }
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(catalog, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.write("\n")
    print(f"{path}: {len(table)} keys")


root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
write_catalog(os.path.join(root, "Offhook/Resources/Localizable.xcstrings"), S)
write_catalog(os.path.join(root, "OffhookIsland/Localizable.xcstrings"), ISLAND)
