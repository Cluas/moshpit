#!/usr/bin/env python3
"""Generate Moshpit/Resources/Localizable.xcstrings and MoshpitIsland/Localizable.xcstrings.

Source language: en (the key IS the English value).
Each entry carries complete zh-Hans + ja translations.
SAME = identical value in every language (brand names, protocol words, glyphs).
"""
import json, os, sys

SAME = object()

# key -> (zh-Hans, ja) | SAME | {"plural": {"en_one":..., "en_other":..., "zh":..., "ja":...}}
S = {}

def add(key, zh=SAME, ja=None):
    S[key] = SAME if zh is SAME else (zh, ja)

def plural(key, en_one, zh, ja):
    S[key] = {"plural": True, "en_one": en_one, "zh": zh, "ja": ja}

# ---------- Brand / protocol / glyphs (identical everywhere) ----------
for k in ["Moshpit", "Vibe Island", "MOSH", "SRTT", "REC",
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
add("Tap ＋ to add your first server. Moshpit keeps your sessions alive across Wi-Fi / 5G handoff.",
    "点按 ＋ 添加第一台服务器。Moshpit 会在 Wi-Fi / 5G 切换时保持会话不断线。",
    "＋をタップして最初のサーバーを追加。Moshpit は Wi-Fi / 5G の切り替えをまたいでセッションを維持します。")
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
add("Mosh keeps the session alive across Wi-Fi / 5G handoff and sleep/wake. UDP must be open server-side; the client picks an unused port within range. With tmux, Moshpit attaches to your existing sessions and never creates or restyles them; only sessions you create through Moshpit get its native look (status bar hidden, restored on disconnect).",
    "Mosh 让会话在 Wi-Fi / 5G 切换及休眠唤醒后保持存活。服务器端需开放 UDP；客户端会在范围内挑选一个未占用的端口。使用 tmux 时，Moshpit 只附加到你已有的会话，绝不创建或改样式；只有通过 Moshpit 创建的会话才会应用其原生外观（隐藏状态栏，断开时恢复）。",
    "Mosh は Wi-Fi / 5G の切り替えやスリープ/復帰をまたいでセッションを維持します。サーバー側で UDP を開放してください。クライアントは範囲内の未使用ポートを選びます。tmux では Moshpit は既存セッションにアタッチするだけで、作成やスタイル変更は行いません。Moshpit から作成したセッションのみネイティブな外観（ステータスバー非表示、切断時に復元）になります。")
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
add("Moshpit watches the active tmux session for the terminal bell and posts a local alert when your agent needs attention.",
    "Moshpit 会监听活动 tmux 会话的终端响铃，并在你的智能体需要关注时发出本地提醒。",
    "Moshpit はアクティブな tmux セッションのターミナルベルを監視し、エージェントが注意を必要とするとローカル通知を出します。")
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
add("Adds a mic key to the terminal bar", "在快捷条上加一个麦克风键", "ショートカットバーにマイクキーを追加します")
add("Dictate commands and prompts from the mic key on the terminal bar. Speech is transcribed entirely on-device — your voice never leaves this device, and nothing is typed until you tap Insert.",
    "用快捷条上的麦克风键口述命令和 prompt。转写完全在本机完成——你的语音绝不离开这台设备，不点「插入」就不会输入任何内容。",
    "ショートカットバーのマイクキーからコマンドやプロンプトを音声入力できます。書き起こしは完全に端末内で行われ、音声がこの端末を離れることはなく、「挿入」をタップするまで何も入力されません。")

# ---------- Voice input · engine + model (Whisper) ----------
add("Recognition", "识别引擎", "認識エンジン")
add("RECOGNITION", "识别引擎", "認識エンジン")
add("Apple (built in)", "Apple（内置）", "Apple（内蔵）")
add("Whisper (local model)", "Whisper（本地模型）", "Whisper（ローカルモデル）")
add("No download. One language per session.", "无需下载。每次会话只识别一种语言。", "ダウンロード不要。1 セッションに 1 言語。")
add("Downloads a model. Around 100 languages, and copes with a sentence that mixes two.",
    "需下载模型。覆盖约 100 种语言，一句话里混用两种也能听准。",
    "モデルのダウンロードが必要。約 100 言語に対応し、1 つの文に 2 言語が混在しても処理できます。")
add("Both engines run entirely on this device — your voice is never uploaded. Whisper additionally needs its model downloaded once over the network before it can be used.",
    "两种引擎都完全在本机运行——你的语音绝不上传。Whisper 另需先联网下载一次模型才能使用。",
    "どちらのエンジンも完全にこの端末で動作し、音声がアップロードされることはありません。Whisper は利用前に一度だけモデルをネットワーク経由でダウンロードする必要があります。")
add("SETUP", "准备", "セットアップ")
add("Download a model", "下载模型", "モデルをダウンロード")
add("Whisper can't transcribe until one is on the device",
    "设备上没有模型时 Whisper 无法转写", "モデルが端末にないと Whisper は書き起こせません")

add("Model", "模型", "モデル")
add("Whisper Model", "Whisper 模型", "Whisper モデル")
add("WHISPER MODEL", "Whisper 模型", "Whisper モデル")
# Variant names stay verbatim: they're the identifiers users match against
# Hugging Face and Whisper's own docs, so translating them would break lookup.
for k in ["Tiny", "Base", "Small", "Large v3 Turbo"]:
    add(k, SAME)
add("Most accurate, especially on mixed-language speech. Needs a recent chip.",
    "准确度最高，尤其是多语种混说。需要较新的芯片。",
    "最も高精度。特に複数言語が混在する音声に強い。新しめのチップが必要です。")
add("Good balance of accuracy and speed.", "准确度与速度较均衡。", "精度と速度のバランスが良い。")
add("Fast and light. Noticeably weaker outside English.",
    "快且占用小。英语以外明显较弱。", "高速・軽量。英語以外は精度が明らかに落ちます。")
add("Fastest, lowest accuracy. For older devices.",
    "最快，准确度最低。适合较旧的设备。", "最速・最低精度。古い端末向け。")
add("≈%@ download", "约 %@ 下载", "約 %@ のダウンロード")
add("Download", "下载", "ダウンロード")
add("Downloading %@", "正在下载 %@", "%@ をダウンロード中")
add("Remove %@", "删除 %@", "%@ を削除")
add("Remove this model?", "删除这个模型？", "このモデルを削除しますか？")
add("Remove", "删除", "削除")
add("Keep", "保留", "残す")
add("%@ frees %@. You can download it again later.",
    "删除 %1$@ 可释放 %2$@。以后可以重新下载。",
    "%1$@ を削除すると %2$@ を解放できます。後で再ダウンロードできます。")
add("Models on this device", "本机已下载", "この端末のモデル")
add("STORAGE", "存储占用", "ストレージ")
add("LAST ERROR", "上次错误", "直近のエラー")
add("Models are downloaded from Hugging Face once, then everything runs on this device — no audio is ever uploaded. Every model here is multilingual; the bigger ones are markedly better outside English and on speech that switches language mid-sentence, but take longer per phrase. Only models this device can run are listed.",
    "模型只从 Hugging Face 下载一次，之后全部在本机运行——音频绝不上传。这里的模型都是多语种的；越大的模型在英语以外、以及一句话中途换语言时明显更准，但每句耗时更长。只列出本机跑得动的模型。",
    "モデルは Hugging Face から一度だけダウンロードされ、その後はすべてこの端末で動作します——音声がアップロードされることはありません。ここにあるモデルはすべて多言語対応で、大きいものほど英語以外や文中で言語が切り替わる音声に強い一方、1 フレーズあたりの処理時間は長くなります。この端末で動作するモデルのみを表示しています。")

# ---------- Voice input · language picker ----------
add("Voice Language", "语音语言", "音声入力の言語")
add("DICTATION LANGUAGE", "听写语言", "音声入力の言語")
add("ALL LANGUAGES", "全部语言", "すべての言語")
add("Automatic", "自动", "自動")
add("Auto-detect", "自动检测", "自動検出")
add("Whisper decides from what it hears", "由 Whisper 根据听到的内容判断", "Whisper が聞こえた内容から判断します")
add("Checking available languages…", "正在检查可用语言…", "利用できる言語を確認中…")
add("Keeps English words mixed into the sentence", "句子里夹的英文词也能保留", "文中に混ざる英単語もそのまま保持します")
add("Languages offered here are the ones this device can transcribe. A language's speech model downloads once on first use, then works offline. Apple's engines handle one language per session — for speech that switches between two, switch Recognition to Whisper.",
    "这里列出的是本机能转写的语言。某个语言的语音模型首次使用时下载一次，之后离线可用。Apple 的引擎每次会话只处理一种语言——如果你说话会在两种语言之间切换，请把「识别引擎」换成 Whisper。",
    "ここに表示されるのは、この端末が書き起こせる言語です。各言語の音声モデルは初回利用時に一度ダウンロードされ、以降はオフラインで動作します。Apple のエンジンは 1 セッションに 1 言語のみ扱うため、2 言語を切り替えて話す場合は「認識エンジン」を Whisper に変更してください。")
add("One model covers around 100 languages. Naming yours transcribes it while keeping the foreign words inside a sentence intact — the English command names and library names you say mid-thought survive. Auto-detect reads the language off the audio instead, which can waver on short or heavily mixed phrases.",
    "一个模型覆盖约 100 种语言。指定你的语言后，句子里夹的外语词也会原样保留——你说到一半冒出来的英文命令名、库名都不会被改掉。「自动检测」则是让模型从音频里判断语言，遇到很短或混得很厉害的句子可能会摇摆。",
    "1 つのモデルで約 100 言語をカバーします。自分の言語を指定すると、文中に混ざる外国語もそのまま保持され、話の途中で出てくる英語のコマンド名やライブラリ名も崩れません。「自動検出」は音声から言語を判定するため、短いフレーズや混在の激しいフレーズでは判定が揺れることがあります。")

# ---------- Voice input · overlay + failures ----------
add("STARTING…", "正在启动…", "開始中…")
add("LISTENING", "正在听", "認識中")
add("FINISHING…", "正在收尾…", "仕上げ中…")
add("INTERRUPTED", "已中断", "中断されました")
add("DOWNLOADING SPEECH MODEL %lld%%", "正在下载语音模型 %lld%%", "音声モデルをダウンロード中 %lld%%")
add("LOADING SPEECH MODEL…", "正在加载语音模型…", "音声モデルを読み込み中…")
add("Insert", "插入", "挿入")
add("Dismiss", "关闭", "閉じる")
add("Open Settings", "打开设置", "設定を開く")
add("Speak — the text lands here first, and Insert types it into the terminal.",
    "说话吧——文字先落在这里，点「插入」才会输入到终端。",
    "話してください——テキストはまずここに表示され、「挿入」をタップするとターミナルに入力されます。")
add("Another app took the microphone before anything was heard.",
    "还没听到内容，麦克风就被别的应用抢走了。",
    "何も認識されないうちに、別のアプリがマイクを使用しました。")
add("Apple · %@", SAME)
add("Auto", "自动", "自動")
# The overlay joins finalized and volatile text with a bare space; it's a
# separator, not prose, and must not gain or lose width in translation.
add(" ", SAME)
add("Apple Dictation · %@", "Apple 听写 · %@", "Apple 音声入力 · %@")
add("Apple Speech · %@", "Apple 语音识别 · %@", "Apple 音声認識 · %@")
add("Unknown", "未知", "不明")
add("Microphone access is off. Enable it in Settings → Privacy → Microphone.",
    "麦克风权限已关闭。请在「设置 → 隐私 → 麦克风」中开启。",
    "マイクへのアクセスがオフです。「設定 → プライバシー → マイク」で許可してください。")
add("Speech recognition is off. Enable it in Settings → Privacy → Speech Recognition.",
    "语音识别权限已关闭。请在「设置 → 隐私 → 语音识别」中开启。",
    "音声認識がオフです。「設定 → プライバシー → 音声認識」で許可してください。")
add("Speech recognition is unavailable.", "语音识别不可用。", "音声認識を利用できません。")
add("Dictation isn't available for %@ on this device.", "本设备不支持 %@ 的听写。", "この端末では %@ の音声入力を利用できません。")
add("No Whisper model is downloaded yet. Pick one in Settings → Voice Input → Model.",
    "还没有下载 Whisper 模型。请在「设置 → 语音输入 → 模型」里选一个。",
    "Whisper モデルがまだダウンロードされていません。「設定 → 音声入力 → モデル」で選んでください。")
add("The %@ speech model isn't downloaded yet.", "%@ 语音模型还没有下载。", "%@ の音声モデルはまだダウンロードされていません。")
add("The %@ speech model didn't download completely. Remove it and try again.",
    "%@ 语音模型没有下载完整。请删除后重试。",
    "%@ の音声モデルを完全にダウンロードできませんでした。削除してから再試行してください。")
add("Couldn't download the speech model: %@", "无法下载语音模型：%@", "音声モデルをダウンロードできませんでした：%@")
add("Couldn't start the microphone: %@", "无法启动麦克风：%@", "マイクを開始できませんでした：%@")
add("No audio input is available.", "没有可用的音频输入。", "利用できる音声入力がありません。")
add("Audio format conversion unavailable.", "音频格式转换不可用。", "音声フォーマットの変換を利用できません。")
add("Audio format conversion failed.", "音频格式转换失败。", "音声フォーマットの変換に失敗しました。")
add("Audio buffer allocation failed.", "音频缓冲区分配失败。", "音声バッファの確保に失敗しました。")
add("16 kHz mono audio is unavailable on this device.",
    "本设备无法提供 16 kHz 单声道音频。", "この端末では 16 kHz モノラル音声を利用できません。")
add("Start voice input", "开始语音输入", "音声入力を開始")
add("Stop voice input", "停止语音输入", "音声入力を停止")
add("Voice input", "语音输入", "音声入力")
add("Accept suggestion and send", "接受建议并发送", "提案を確定して送信")
add("Return", "回车", "Return")

# ---------- Behavior ----------
add("Keyboard on Open", "打开时弹出键盘", "開いたらキーボードを表示")
add("Raise the keyboard as soon as a terminal opens, instead of after you tap it",
    "打开终端就弹出键盘，而不是等你点一下之后",
    "タップを待たず、ターミナルを開いた時点でキーボードを表示します")
# Notification info sheet
add("Bell = attention", "响铃 = 需要关注", "ベル = 要注意")
add("When a tmux pane rings the terminal bell (BEL) — which Claude Code and most CLIs emit when they finish or need input — Moshpit posts a local notification and flips the Vibe Island to “needs attention.”",
    "当 tmux 窗格触发终端响铃（BEL）——Claude Code 和多数 CLI 在完成或需要输入时都会发出——Moshpit 会发送本地通知，并把 Vibe Island 切换为“需要关注”。",
    "tmux ペインがターミナルベル（BEL）を鳴らすと——Claude Code やほとんどの CLI は完了時や入力待ちで発します——Moshpit はローカル通知を送り、Vibe Island を「要注意」に切り替えます。")
add("While a session is attached, the Dynamic Island shows whether the agent is working, idle, or waiting on you. Tapping it deep-links straight back to that pane.",
    "会话附加期间，灵动岛会显示智能体正在工作、空闲，还是在等你。点按即可直接深链回到该窗格。",
    "セッションのアタッチ中、Dynamic Island はエージェントが作業中・アイドル・あなた待ちのいずれかを表示します。タップするとそのペインへ直接ジャンプします。")
add("Local, not push", "本地通知，而非推送", "ローカル通知（プッシュではありません）")
add("Alerts are generated on-device from the live session — there’s no cloud push server. They fire while Moshpit is in the foreground or recently backgrounded; a fully suspended app won’t poll. iOS will ask for notification permission the first time you connect with Notifications on.",
    "提醒由设备端的实时会话生成——没有云端推送服务器。它们在 Moshpit 处于前台或刚进入后台时触发；完全挂起的应用不会轮询。首次在开启通知的情况下连接时，iOS 会请求通知权限。",
    "通知はライブセッションからデバイス上で生成されます——クラウドのプッシュサーバーはありません。Moshpit がフォアグラウンドまたは直近バックグラウンドの間に発火し、完全にサスペンドされたアプリはポーリングしません。通知をオンにして初めて接続する際、iOS が通知の許可を求めます。")
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
add("This server has no running sessions.\nMoshpit only attaches — create the first one to start.",
    "该服务器上没有正在运行的会话。\nMoshpit 只做附加 — 创建第一个会话以开始。",
    "このサーバーには実行中のセッションがありません。\nMoshpit はアタッチのみ行います — 最初のセッションを作成して始めましょう。")
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
    """Merge the curated table INTO the existing catalog.

    Merge, not overwrite. Two things write this file: this script (the curated
    translations below) and Xcode, which appends every key it extracts from the
    source as you build in the IDE. A wholesale rewrite here deleted everything
    Xcode had found — silently, since the file is generated and nobody reads the
    diff — and the next IDE build put the keys back untranslated, which is why
    this file was permanently dirty. Curated entries win; anything else is left
    exactly as it was.
    """
    existing = {}
    if os.path.exists(path):
        try:
            with open(path, encoding="utf-8") as f:
                existing = json.load(f).get("strings") or {}
        except (json.JSONDecodeError, OSError) as exc:
            raise SystemExit(f"{path}: refusing to overwrite unreadable catalog ({exc})")

    merged = dict(existing)
    for key, spec in table.items():
        merged[key] = entry(key, spec)

    catalog = {"sourceLanguage": "en", "strings": merged, "version": "1.0"}
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(catalog, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.write("\n")
    kept = len(merged) - len(table)
    print(f"{path}: {len(table)} curated + {kept} kept = {len(merged)} keys")


def translated(loc):
    """Does this catalog entry carry a real translation for a language?"""
    unit_ = loc.get("stringUnit") or {}
    if unit_.get("value"):
        return True
    # Plurals keep their values one level down, under variations.
    return bool(loc.get("variations"))


def check(paths):
    """Report every string the app needs that has no zh-Hans / ja translation.

    Reads the keys the compiler actually extracted (`.stringsdata` emitted by
    SWIFT_EMIT_LOC_STRINGS) rather than grepping the source, so interpolation
    is already normalized to %@ / %lld and nothing is missed by a regex that
    didn't anticipate a call shape.
    """
    import glob
    import plistlib

    def load_stringsdata(path):
        """Xcode writes these as JSON today and as a binary plist historically."""
        with open(path, "rb") as f:
            head = f.read(8)
        try:
            if head.startswith(b"bplist"):
                with open(path, "rb") as f:
                    return plistlib.load(f)
            with open(path, encoding="utf-8") as f:
                return json.load(f)
        except Exception:
            return None

    needed = {}
    pattern = os.path.expanduser(
        "~/Library/Developer/Xcode/DerivedData/Moshpit-*/Build/Intermediates.noindex"
        "/*/Debug-*/*/Objects-normal/*/*.stringsdata")
    for found in glob.glob(pattern):
        if "AppShortcuts" in found:
            continue
        data = load_stringsdata(found)
        if data is None:
            continue
        source = data.get("source", "")
        if "/code/moshi/" not in source:
            continue
        for entries in (data.get("tables") or {}).values():
            for item in entries:
                if item.get("key"):
                    needed[item["key"]] = source.split("/code/moshi/")[-1]
    if not needed:
        raise SystemExit("check: no .stringsdata found — build the app first")

    have = {}
    for path in paths:
        if not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as f:
            have.update(json.load(f).get("strings") or {})

    gaps = []
    for key, source in sorted(needed.items()):
        locs = (have.get(key) or {}).get("localizations") or {}
        missing = [lang for lang in ("zh-Hans", "ja")
                   if not translated(locs.get(lang) or {})]
        if missing:
            gaps.append((key, source, missing))

    for key, source, missing in gaps:
        print(f"{source}: {'/'.join(missing)}\t{key}")
    print(f"\n{len(needed)} keys used, {len(gaps)} without a full translation")
    return 1 if gaps else 0


root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CATALOGS = [os.path.join(root, "Moshpit/Resources/Localizable.xcstrings"),
            os.path.join(root, "MoshpitIsland/Localizable.xcstrings")]

if "--check" in sys.argv:
    raise SystemExit(check(CATALOGS))

write_catalog(CATALOGS[0], S)
write_catalog(CATALOGS[1], ISLAND)
