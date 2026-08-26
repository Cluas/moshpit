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
add("Resume", "继续", "再開")
add("Pause", "暂停", "一時停止")
add("%@ of ≈%@ — paused", "已下载 %1$@ / 约 %2$@ — 已暂停", "%1$@ / 約 %2$@ — 一時停止中")
add("Discard the partial download of %@", "丢弃 %@ 已下载的部分", "%@ の途中までのダウンロードを破棄")
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
add("Speak — the text lands here first. Insert types it; Send types it and presses Return.",
    "说话吧——文字先落在这里。「插入」只输入，「发送」输入并按回车。",
    "話してください——テキストはまずここに表示されます。「挿入」は入力のみ、「送信」は入力して Return を押します。")
add("Send", "发送", "送信")
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
# Offline host-setup sheet
add("Automatic — connect to inspect", "自动完成——连接后可查看", "自動——接続すると確認できます")
add("Host setup is automatic. Connecting to a host installs and repairs everything it needs — scripts, hooks registration, push pairing — and asks before its first install. Connect to a host to inspect or remove its setup here.",
    "主机设置是自动的。连接主机时会自动安装并修复所需的一切——脚本、钩子注册、推送配对——首次安装前会先询问你。连接主机后可在此查看或移除其设置。",
    "ホスト設定は自動です。ホストに接続すると必要なもの——スクリプト、フック登録、プッシュペアリング——を自動でインストール・修復し、初回インストール前には確認します。接続するとここで確認・削除できます。")
add("PAIRED HOSTS", "已配对主机", "ペアリング済みホスト")
add("paired %@", "配对于 %@", "%@ にペアリング")
# Notification info sheet
add("Agents stamp their state", "智能体自报状态", "エージェントが状態を報告")
add("Coding agents (Claude Code, Codex, …) report working / needs-you / done through hooks Moshpit installs on your host — precise states, not guesses. The terminal bell still works as a fallback for everything else.",
    "编码智能体（Claude Code、Codex 等）通过 Moshpit 安装在主机上的钩子上报「工作中 / 等你 / 已完成」——精确状态，不靠猜。其余程序仍以终端响铃作为兜底。",
    "コーディングエージェント（Claude Code、Codex など）は Moshpit がホストにインストールするフックで「作業中 / あなた待ち / 完了」を報告します——推測ではなく正確な状態です。それ以外はターミナルベルがフォールバックとして機能します。")
add("While a session is attached, the Dynamic Island shows whether the agent is working, idle, or waiting on you. Tapping it deep-links straight back to that pane.",
    "会话附加期间，灵动岛会显示智能体正在工作、空闲，还是在等你。点按即可直接深链回到该窗格。",
    "セッションのアタッチ中、Dynamic Island はエージェントが作業中・アイドル・あなた待ちのいずれかを表示します。タップするとそのペインへ直接ジャンプします。")
add("Push, sealed end-to-end", "推送，端到端加密", "プッシュ、エンドツーエンド暗号化")
add("When the app isn’t running, your host sends the alert through Moshpit’s push relay. It is encrypted on your host with a key only this device holds — the relay and Apple carry ciphertext and can read none of it. Delivered even from a locked phone.",
    "应用未运行时，提醒由你的主机经 Moshpit 推送中继送达。内容在你的主机上用只有这台设备持有的密钥加密——中继和 Apple 只经手密文，谁也读不了。手机锁屏也能送达。",
    "アプリが起動していないとき、通知はホストから Moshpit のプッシュリレー経由で届きます。内容はこのデバイスだけが持つ鍵でホスト上で暗号化され、リレーも Apple も暗号文を運ぶだけで読めません。ロック中の iPhone にも届きます。")
add("Quiet by design", "为安静而设计", "静けさを前提に")
add("A question must stand for 30 seconds before any phone hears about it — answered at your desk means never announced. All waiting agents share one summary card; only the first rings. A finished turn only chimes if it ran three minutes or more. Parked agents stay silent.",
    "一个提问要站立满 30 秒手机才会知道——在桌面上顺手答掉就永远不响。所有等待中的智能体共用一张摘要卡，只有第一个会响铃。任务完成只有跑满三分钟才会提示音。待机的智能体保持静默。",
    "問いかけは 30 秒間続いて初めて通知されます——デスクですぐ答えれば鳴りません。待機中のエージェントは 1 枚のサマリーカードを共有し、鳴るのは最初の 1 回だけ。完了音は 3 分以上かかったターンのみ。放置中のエージェントは静かなままです。")
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

# ---------- Backfill ----------
# Strings that shipped untranslated: they were in the app but never in this
# table, so a zh-Hans or ja user saw English. Found with `--check`, which reads
# the keys the compiler extracted rather than trusting this file to be complete.
# Kept as one block per screen rather than merged into the sections above so the
# backfill stays reviewable as a unit.

# Home (AttachHomeView)
add("THE PIT NEVER CLOSES", "机器不眠", "ピットは閉じない")
add("the pit never closes · your sessions wait for you",
    "机器不眠 · 会话一直等着你", "ピットは閉じない · セッションはあなたを待っています")
add("SSH", SAME)
add("AGENTS", "智能体", "エージェント")
add("OPEN", "打开", "開く")
add("RETRY", "重试", "再試行")
add("WAIT", "等待", "待機")
add("%@", SAME)
add("no hosts yet", "还没有主机", "ホストがありません")
add("No %@ yet", "还没有 %@", "%@ がありません")
add("saved", "已保存", "保存済み")
add("live", "在线", "接続中")
add("linking", "连接中", "接続処理中")
add("stalled", "已停滞", "停滞")
add("now", "刚刚", "たった今")
add("agent", "智能体", "エージェント")
add("1 host saved · all quiet", "已保存 1 台主机 · 一切安静", "ホスト 1 台を保存 · 静かです")
add("%lld hosts saved · all quiet", "已保存 %lld 台主机 · 一切安静", "ホスト %lld 台を保存 · 静かです")
add("1 live connection · agents quiet", "1 个在线连接 · 智能体安静", "接続 1 件 · エージェントは静かです")
add("%lld live connections · agents quiet",
    "%lld 个在线连接 · 智能体安静", "接続 %lld 件 · エージェントは静かです")
add("1 agent needs you", "1 个智能体需要你", "エージェント 1 体があなたを待っています")
add("%lld agents need you", "%lld 个智能体需要你", "エージェント %lld 体があなたを待っています")
add("%lld NEED YOU", "%lld 个待处理", "%lld 件が要対応")
add("Connection lost — tap to reconnect", "连接已断开——点按重连", "接続が切れました——タップで再接続")
add("Disconnecting…", "正在断开…", "切断中…")
add("Attach didn't complete — tmux never confirmed.",
    "附加未完成——tmux 始终没有确认。", "アタッチが完了しませんでした——tmux の確認が返りませんでした。")
add("Nothing running — start a task to isolate one",
    "没有在跑的任务——新建一个任务来隔离出工作区", "実行中のものはありません——タスクを開始して分離してください")
add("Leave blank for an automatic name.", "留空则自动命名。", "空欄にすると自動で名前が付きます。")
add("Remove Worktree", "删除工作树", "ワークツリーを削除")
add("Remove the worktree for \"%@\"?", "删除 “%@” 的工作树？", "「%@」のワークツリーを削除しますか？")
add("Deletes the branch checkout under ~/.herdr/worktrees. %@ itself is untouched.",
    "只删除 ~/.herdr/worktrees 下的分支检出，%@ 本身不受影响。",
    "~/.herdr/worktrees 配下のブランチのチェックアウトのみを削除します。%@ 自体はそのままです。")
add("\"%@\" has uncommitted changes", "“%@” 有未提交的改动", "「%@」に未コミットの変更があります")
add("Those changes exist nowhere else. Removing the worktree throws them away.",
    "这些改动别处没有副本。删掉工作树就等于丢弃它们。",
    "これらの変更は他のどこにも存在しません。ワークツリーを削除すると失われます。")
add("Delete anyway", "仍然删除", "それでも削除")
add("Keep it", "保留", "残す")
add("Couldn't remove the worktree", "无法删除工作树", "ワークツリーを削除できませんでした")
add("%@ %@ \"%@\"?", SAME)
add("%@ pane %lld?", "%@ 窗格 %lld？", "%@ ペイン %lld？")

# Terminal
add("Opening the pit", "正在打开", "接続中")
add("Riding the handoff", "正在跨网切换", "ハンドオフ中")
add("mosh keeps the line up · sessions survive the handoff",
    "mosh 保持链路 · 会话扛得住网络切换", "mosh が接続を維持 · セッションはハンドオフを乗り切ります")
add("%@@%@:%lld", SAME)
add("%llu", SAME)
add("Attaching %@…", "正在附加到 %@…", "%@ にアタッチ中…")
add("ctrl", SAME)
add("Control", "Control 键", "Control キー")
add("Control armed", "Control 已就绪", "Control キー待機中")
add("Paste", "粘贴", "貼り付け")
add("Scroll up", "向上滚动", "上にスクロール")
add("Scroll down", "向下滚动", "下にスクロール")
add("Scroll history", "滚动历史", "履歴をスクロール")
add("Drag up or down to scroll the terminal scrollback",
    "上下拖动可滚动终端历史", "上下にドラッグしてターミナルの履歴をスクロールします")
add("Double tap to switch between SSH and Mosh",
    "双击可在 SSH 与 Mosh 之间切换", "ダブルタップで SSH と Mosh を切り替えます")
add("Switch", "切换", "切り替え")
add("Switch to %@?", "切换到 %@？", "%@ に切り替えますか？")
add("Reconnects this session over %@.", "用 %@ 重新连接这个会话。", "このセッションを %@ で再接続します。")
add("Line dropped — retrying", "链路中断——正在重试", "接続が切れました——再試行中")
add("MOSH DIAGNOSTICS", "MOSH 诊断", "MOSH 診断")
add("No datagrams yet.", "还没有收到数据报。", "まだデータグラムを受信していません。")
add("%@ not installed on this host", "这台主机上没装 %@", "このホストに %@ がインストールされていません")
add("Install %@", "安装 %@", "%@ をインストール")
add("Create %@", "新建 %@", "%@ を作成")
add("This server has no running %@.\nMoshpit only attaches — create the first one to start.",
    "这台服务器上没有正在运行的 %@。\nMoshpit 只做附加——先创建第一个才能开始。",
    "このサーバーで実行中の %@ はありません。\nMoshpit はアタッチのみ行います——まず 1 つ作成してください。")
add("No %@ %@", "没有 %@ %@", "%@ %@ がありません")
add("Moshpit needs %@ for %@ navigation.\nInstall it, then reconnect.",
    "%@ 导航需要 %@。\n装好之后重新连接。",
    "%@ のナビゲーションには %@ が必要です。\nインストール後に再接続してください。")

# Theme editor. The preview pane paints a MOCK terminal session — real git
# output, a prompt glyph, a block cursor. Every one of those stays verbatim:
# translating simulated `git status` output would misrepresent what a terminal
# shows, and the preview exists precisely to be checked against reality.
for k in ["git ", "status", "main", "On branch ", "warning: ", "hint: use --staged",
          "MoshTransport.swift", "MoshpitMark.swift", "2 files changed",
          "❯ ", "❯", "█", "+ ", "- "]:
    add(k, SAME)
add("TERMINAL", "终端", "ターミナル")
add("Theme name", "主题名称", "テーマ名")
add("Theme preview", "主题预览", "テーマのプレビュー")
add("ANSI COLORS", "ANSI 颜色", "ANSI カラー")
add("BRIGHT COLORS", "高亮颜色", "明るいカラー")
add("These eight are what shells, diffs and TUIs paint with.",
    "shell、diff 和 TUI 用的就是这八种颜色。",
    "シェル・diff・TUI が使うのはこの 8 色です。")
add("Override bright colors", "自定义高亮颜色", "明るいカラーを上書き")
add("Currently derived automatically", "当前自动推导", "現在は自動で算出")
add("Bright slots are derived from the eight above by default. Override them only if you want exact control — many tools paint dim text with bright black, so keeping it distinct from black matters.",
    "高亮档位默认由上面八色推导。只有你想精确控制时才需要自定义——很多工具用「亮黑」画灰字，所以它和纯黑必须能区分开。",
    "明るいカラーは既定で上記 8 色から算出されます。厳密に制御したい場合のみ上書きしてください——多くのツールは淡色テキストを明るい黒で描くため、黒と区別できることが重要です。")
add("Copy as JSON", "复制为 JSON", "JSON としてコピー")

# Theme gallery
add("BUILT-IN", "内置", "内蔵")
add("MY THEMES", "我的主题", "マイテーマ")
add("New Theme", "新建主题", "新規テーマ")
add("Add theme", "添加主题", "テーマを追加")
add("Edit %@", "编辑 %@", "%@ を編集")
add("Duplicate", "复制", "複製")
add("Duplicate & Edit", "复制并编辑", "複製して編集")
add("Copy JSON", "复制 JSON", "JSON をコピー")
add("Import JSON…", "导入 JSON…", "JSON を読み込む…")
add("Import Theme", "导入主题", "テーマを読み込む")
add("Paste from Clipboard", "从剪贴板粘贴", "クリップボードから貼り付け")
add("No custom themes yet.", "还没有自定义主题。", "カスタムテーマはまだありません。")
add("This theme will be removed. Built-in themes are unaffected.",
    "该主题会被移除。内置主题不受影响。", "このテーマを削除します。内蔵テーマには影響しません。")
add("Duplicate a built-in theme to start from its palette, or import one as JSON. Long-press a theme for more.",
    "复制一个内置主题即可基于它的配色开始，也可以导入 JSON。长按主题可看到更多操作。",
    "内蔵テーマを複製してその配色から始めるか、JSON で読み込みます。テーマを長押しすると他の操作が表示されます。")
add("Paste a theme exported from Moshpit, or any JSON object with \"background\", \"foreground\" and the eight ANSI color names (\"black\", \"red\", …). Bright colors are optional.",
    "粘贴从 Moshpit 导出的主题，或任何带 “background”、“foreground” 和八个 ANSI 颜色名（“black”、“red” …）的 JSON 对象。高亮颜色可选。",
    "Moshpit から書き出したテーマ、または \"background\"・\"foreground\" と 8 つの ANSI カラー名（\"black\"、\"red\" …）を持つ JSON オブジェクトを貼り付けます。明るいカラーは任意です。")

# Accent gallery
add("MY ACCENTS", "我的强调色", "マイアクセント")
add("New accent", "新建强调色", "新規アクセント")
add("Accent name", "强调色名称", "アクセント名")
add("Accent preview", "强调色预览", "アクセントのプレビュー")
add("NAME", "名称", "名前")
add("COLOR", "颜色", "カラー")
add("CONNECT", "连接", "接続")
add("No custom accents yet.", "还没有自定义强调色。", "カスタムアクセントはまだありません。")
add("The pressed state and the background wash are derived from this one color.",
    "按下态和背景微光都由这一个颜色推导。", "押下時の状態と背景のごく淡い色は、この 1 色から算出されます。")
add("This accent will be removed. Built-in accents are unaffected.",
    "该强调色会被移除。内置强调色不受影响。", "このアクセントを削除します。内蔵アクセントには影響しません。")
add("A custom accent tints controls, highlights and the faint background wash. Status colors (warning, success, error) stay fixed so they never get mistaken for the accent.",
    "自定义强调色会作用于控件、高亮和背景微光。状态色（警告、成功、错误）保持固定，避免和强调色混淆。",
    "カスタムアクセントはコントロール・ハイライト・背景のごく淡い色に適用されます。ステータス色（警告・成功・エラー）は固定で、アクセントと混同されることはありません。")
add("selected row", "选中行", "選択中の行")
add("ok", SAME)
add("warn", "警告", "警告")
add("err", "错误", "エラー")

# New agent task (herdr)
add("New Agent Task", "新建智能体任务", "新規エージェントタスク")
add("TASK", "任务", "タスク")
add("Repo", "仓库", "リポジトリ")
add("Repository path", "仓库路径", "リポジトリのパス")
add("Branch", "分支", "ブランチ")
add("Agent", "智能体", "エージェント")
add("Prompt", "提示词", "プロンプト")
add("FIRST MESSAGE", "首条消息", "最初のメッセージ")
add("Choose", "选择", "選択")
add("Custom", "自定义", "カスタム")
add("Other…", "其他…", "その他…")
add("Start", "开始", "開始")
add("Starting…", "正在启动…", "開始中…")
add("Looking for repositories…", "正在查找仓库…", "リポジトリを検索中…")
add("None found — no panes in repos, and nothing under ~",
    "没找到——窗格都不在仓库里，主目录下也没有", "見つかりません——リポジトリ内のペインがなく、ホーム直下にもありません")
add("Creates a git worktree on the host, then starts the agent inside it. Your working tree is untouched.",
    "在主机上创建一个 git worktree，然后在里面启动智能体。你的工作树不受影响。",
    "ホスト上に git ワークツリーを作成し、その中でエージェントを起動します。あなたの作業ツリーはそのままです。")
add("Optional. Sent to the agent once it's running — leave blank to type it yourself.",
    "可选。智能体启动后会发给它——留空则你自己手输。",
    "任意。エージェントの起動後に送信されます——空欄にすると自分で入力できます。")

# Settings
add("APPEARANCE", "外观", "外観")
add("Accent", "强调色", "アクセント")
add("App Icon", "应用图标", "アプリアイコン")
add("Language", "语言", "言語")
add("Copy", "复制", "コピー")
add("The accent color tints the app's controls and highlights. The home-screen icon is a separate choice. Both are separate from the terminal color scheme (Display → Theme, below).",
    "强调色作用于 App 的控件和高亮。主屏图标是另一项独立选择。两者都与终端配色无关（终端配色在下面的「显示 → 主题」）。",
    "アクセントカラーはアプリのコントロールとハイライトに適用されます。ホーム画面のアイコンは別の設定です。どちらもターミナルの配色（下の「表示 → テーマ」）とは無関係です。")
add("Leaves a signal trail behind the cursor while mosh predicts ahead of the server",
    "mosh 抢先于服务器预测时，在光标后留下一道轨迹",
    "mosh がサーバーより先に予測している間、カーソルの後ろに軌跡を残します")
add("Alert when an agent needs you", "智能体需要你时提醒", "エージェントがあなたを必要とするとき通知")
add("Alert sound", "提示音", "通知音")
add("Play a sound when the agent needs you", "智能体需要你时播放提示音", "エージェントがあなたを必要とするとき音を鳴らします")
add("Show detail on lock screen", "在锁屏上显示详情", "ロック画面に詳細を表示")
add("Display what the agent is running/asking — off keeps it private",
    "显示智能体正在跑什么、在问什么——关掉则保持私密",
    "エージェントが実行中の内容や問いかけを表示します——オフにすると非表示のままです")
add("Moshpit watches the active session for agent activity and posts a local alert when your agent needs attention — natively on herdr, via the bell and hooks on tmux.",
    "Moshpit 会盯着当前会话里的智能体活动，需要你处理时发本地通知——herdr 上是原生支持，tmux 上靠响铃和 hook。",
    "Moshpit はアクティブなセッションのエージェントの動きを監視し、対応が必要になるとローカル通知を送ります——herdr ではネイティブに、tmux ではベルとフックを介して行います。")

# Add Connection
add("Multiplexer", "多路复用器", "マルチプレクサ")
add("mosh-server path", "mosh-server 路径", "mosh-server のパス")
add("Custom herdr Path", "自定义 herdr 路径", "herdr のカスタムパス")
add("tmux and herdr hold separate, unrelated sessions. If the host doesn't have the one you pick, Moshpit says so and drops to a plain shell — it never quietly attaches the other. With Mosh, herdr runs its own terminal UI; native rendering needs SSH.",
    "tmux 和 herdr 各自持有互不相关的会话。如果主机上没有你选的那个，Moshpit 会明确告知并退回普通 shell——绝不会悄悄连上另一个。搭配 Mosh 时 herdr 跑它自己的终端界面；原生渲染需要 SSH。",
    "tmux と herdr はそれぞれ独立した無関係のセッションを保持します。選んだ方がホストにない場合、Moshpit はそれを明示して通常のシェルに切り替えます——もう一方に黙ってアタッチすることはありません。Mosh と併用する場合、herdr は独自のターミナル UI を表示します。ネイティブ描画には SSH が必要です。")
# SOCKS5 proxy
add("PROXY", "代理", "プロキシ")
add("Use SOCKS5 Proxy", "使用 SOCKS5 代理", "SOCKS5 プロキシを使用")
add("Route this connection through a local or corporate SOCKS5 proxy",
    "让这个连接走本地或公司的 SOCKS5 代理",
    "この接続をローカルまたは社内の SOCKS5 プロキシ経由にします")
add("Proxy Host", "代理主机", "プロキシのホスト")
add("Proxy Port", "代理端口", "プロキシのポート")
add("1080", SAME)
add("Only unauthenticated SOCKS5 proxies are supported. This proxies the SSH connection only — if Mosh is also enabled above, its UDP session connects directly once bootstrapped and is not proxied.",
    "仅支持免认证的 SOCKS5 代理。它只代理 SSH 连接——如果上面同时开了 Mosh，其 UDP 会话在建立后直连，不走代理。",
    "認証なしの SOCKS5 プロキシのみ対応しています。プロキシ経由になるのは SSH 接続のみです——上で Mosh も有効な場合、その UDP セッションは確立後は直接接続され、プロキシを経由しません。")

# Agent notifications (actionable, from the Island / lock screen)
add("Allow", "允许", "許可")
add("Deny", "拒绝", "拒否")
add("Reply", "回复", "返信")
add("Send", "发送", "送信")
add("Stop", "停止", "停止")
add("Type a response…", "输入回复…", "返信を入力…")
add("Next instruction…", "下一条指令…", "次の指示…")
add("Not delivered", "未送达", "送信されませんでした")
add("Your tap didn't reach the agent — open Moshpit and answer there.",
    "你的点按没有送达智能体——请打开 Moshpit 在里面回应。",
    "タップがエージェントに届きませんでした——Moshpit を開いて操作してください。")
add("Sent to your Mac", "已发给你的 Mac", "Mac に送信しました")
add("Your answer was passed to the host. It takes effect if the agent is still waiting on that question.",
    "你的回答已交给主机。若智能体仍停在那个提问上，回答就会生效。",
    "回答をホストに渡しました。エージェントがまだその問いで待っていれば反映されます。")
add("Stop was passed to the host. It takes effect if the agent is still running.",
    "停止指令已交给主机。若智能体仍在运行，指令就会生效。",
    "停止をホストに渡しました。エージェントがまだ実行中であれば反映されます。")
add("Enable agent notifications on %@?",
    "在 %@ 上启用智能体通知？",
    "%@ でエージェント通知を有効にしますか？")
add("Enable", "启用", "有効にする")
add("Not Now", "以后再说", "今はしない")
add("Don't Ask Again", "不再询问", "今後確認しない")
add("Moshpit installs its hook scripts in ~/.moshpit and registers them in Claude Code's settings, so agents can reach you when they need you. Everything can be removed from Host Setup.",
    "Moshpit 会把钩子脚本安装到 ~/.moshpit 并注册进 Claude Code 的设置，这样智能体需要你时能找到你。所有内容都可以在主机设置里移除。",
    "Moshpit は ~/.moshpit にフックスクリプトをインストールし、Claude Code の設定に登録します。エージェントがあなたを必要とするとき届くようにするためです。すべてホスト設定から削除できます。")
add("Prompt already gone", "该提问已不存在", "問いかけは既にありません")
add("That request was already answered or has changed — nothing was sent. Open Moshpit to see the current state.",
    "那个请求已经被回应过、或者状态变了——什么都没发出去。打开 Moshpit 看当前状态。",
    "そのリクエストは既に回答済みか変化しています——何も送信されていません。Moshpit を開いて現在の状態を確認してください。")

# Island hooks installer
add("AGENT", "智能体", "エージェント")
add("Not run", "未运行", "未実行")
add("Run an agent turn in any pane, then re-check.",
    "在任意窗格里跑一轮智能体，然后重新检查。", "任意のペインでエージェントを 1 ターン実行し、再確認してください。")
add("Backs up your config", "会备份你的配置", "設定をバックアップします")
add("Copies the agent's config to a timestamped backup before merging Moshpit's hook groups.",
    "在合入 Moshpit 的 hook 组之前，先把智能体配置复制成带时间戳的备份。",
    "Moshpit のフックを統合する前に、エージェントの設定をタイムスタンプ付きでバックアップします。")
add("Never blocks the agent", "绝不阻塞智能体", "エージェントを妨げません")
add("The hooks only stamp the tmux pane and exit 0 — the agent is never slowed, prompted, or interrupted.",
    "这些 hook 只在 tmux 窗格上打个标记然后 exit 0——不会让智能体变慢、不会弹提示、也不会打断它。",
    "フックは tmux ペインに印を付けて exit 0 するだけです——エージェントが遅くなったり、確認を求められたり、中断されることはありません。")
add("Idempotent: re-running de-dupes Moshpit's hooks instead of stacking them.",
    "幂等：重复运行会去重 Moshpit 的 hook，而不是层层堆叠。",
    "冪等です。再実行すると Moshpit のフックは重複除去され、積み重なりません。")
add("Edits %@ (a timestamped backup is written first).",
    "会修改 %@（先写一份带时间戳的备份）。", "%@ を編集します（先にタイムスタンプ付きのバックアップを書き出します）。")
add("Install Moshpit's hooks so the Vibe Island shows exactly when your agent is working, what it's running, when it needs you, and when it's done — instead of guessing from output. Moshpit backs up your config first and never blocks the agent.",
    "安装 Moshpit 的 hook，让 Vibe Island 准确显示智能体何时在工作、在跑什么、何时需要你、何时完成——而不是靠输出去猜。Moshpit 会先备份你的配置，且绝不阻塞智能体。",
    "Moshpit のフックをインストールすると、Vibe Island はエージェントがいつ作業中か、何を実行しているか、いつあなたを必要としているか、いつ完了したかを正確に表示します——出力から推測する必要はありません。Moshpit は先に設定をバックアップし、エージェントを妨げることはありません。")

# Host key verification (Components)
add("%lld", SAME)
add("active", "使用中", "使用中")
add("SHA256 fingerprint", "SHA256 指纹", "SHA256 フィンガープリント")
add("Stored", "已存储", "保存済み")
add("Offered now", "本次提供", "今回提示された値")
add("Verify on server", "在服务器上核对", "サーバー側で確認")
add("Host Key Changed", "主机密钥已变更", "ホストキーが変更されました")
add("First connection to %@:%@. Verify this fingerprint matches the server before you trust it.",
    "首次连接 %@:%@。信任之前请先核对这个指纹与服务器一致。",
    "%@:%@ への初回接続です。信頼する前に、このフィンガープリントがサーバーと一致することを確認してください。")
add("The key for %@:%@ does **not** match what's stored here. The server may have been reinstalled — or the connection is being intercepted.",
    "%@:%@ 的密钥与此处存储的**不一致**。可能是服务器重装了——也可能连接正被中间人截获。",
    "%@:%@ のキーは、ここに保存されているものと**一致しません**。サーバーが再インストールされた可能性——あるいは接続が傍受されている可能性があります。")

# Multiplexer vocabulary (tmux vs herdr wording)
add("Session", "会话", "セッション")
add("Window", "窗口", "ウィンドウ")
add("Workspace", "工作区", "ワークスペース")
add("Workspaces", "工作区", "ワークスペース")
add("Tab", "标签页", "タブ")
add("Tabs", "标签页", "タブ")
add("Kill", "终止", "強制終了")
add("Close", "关闭", "閉じる")
add("＋ splits a new pane", "＋ 新建一个窗格", "＋ で新しいペインを分割")

# Branch-name validation (new agent task)
add("Pick a repository", "选一个仓库", "リポジトリを選択してください")
add("Pick an agent", "选一个智能体", "エージェントを選択してください")
add("Name the branch", "给分支起个名字", "ブランチ名を入力してください")
add("No spaces in a branch name", "分支名不能有空格", "ブランチ名に空白は使えません")
add("No control characters in a branch name", "分支名不能有控制字符", "ブランチ名に制御文字は使えません")
add("No “..” in a branch name", "分支名不能含 “..”", "ブランチ名に「..」は使えません")
add("No ~ ^ : ? * [ \\ in a branch name", "分支名不能含 ~ ^ : ? * [ \\", "ブランチ名に ~ ^ : ? * [ \\ は使えません")
add("Can't start with “-” or “/”, or end with “/”",
    "不能以 “-” 或 “/” 开头，也不能以 “/” 结尾", "「-」「/」で始めたり、「/」で終わることはできません")
add("Can't end with “.lock”", "不能以 “.lock” 结尾", "「.lock」で終わることはできません")

# tmux / herdr sheets. The nouns are substituted from MultiplexerVocabulary
# (window vs tab, session vs workspace), so the verb has to sit in an order
# that reads correctly for either.
add("%@ %@", SAME)
add("New %@", "新建 %@", "新規 %@")
add("Rename %@", "重命名 %@", "%@ の名前を変更")
add("Leave blank to let the program name it.", "留空则由程序命名。", "空欄にするとプログラムが名前を付けます。")
add("Tap to switch", "点按切换", "タップで切り替え")
add("Tap to focus · %@", "点按聚焦 · %@", "タップでフォーカス · %@")
add("%@ %@ %@? Every pane in it dies.", "%1$@ %2$@ %3$@？其中每个窗格都会终止。",
    "%1$@ %2$@ %3$@？その中のすべてのペインが終了します。")
add("%@ %@ %@? Everything in it dies.", "%1$@ %2$@ %3$@？其中的一切都会终止。",
    "%1$@ %2$@ %3$@？その中のすべてが終了します。")

# Island controls (accessibility labels on the Live Activity buttons)
add("Action", "操作", "操作")
add("Connection", "连接", "接続")
add("Pane", "窗格", "ペイン")
add("Respond to agent", "回应智能体", "エージェントに応答")
add("Switch agent", "切换智能体", "エージェントを切り替え")
add("✓ %@ finished", "✓ %@ 已完成", "✓ %@ が完了しました")

# APNs fallback text. These are not shown by any Swift code: the push relay puts
# them in the payload as `title-loc-key` / `loc-key`, and iOS resolves them
# against this catalog. They surface only when the notification service
# extension fails to decrypt in time — see docs/PUSH.md. The keys ARE the
# English, so a missed lookup still reads as a sentence.
add("An agent needs you", "有 agent 在等你", "エージェントが待っています")
add("Open Moshpit to see what it is asking.",
    "打开 Moshpit 看它在问什么。", "Moshpit を開いて内容を確認してください。")
add("An agent finished", "有 agent 已完成", "エージェントが完了しました")
add("Open Moshpit to send the next instruction.",
    "打开 Moshpit 发下一条指令。", "Moshpit を開いて次の指示を送ってください。")
add("pane %lld", "窗格 %lld", "ペイン %lld")

# Multiplexer choice (Add Connection)
add("None", "不使用", "使用しない")
add("Single shell, no session persistence", "单个 shell，会话不持久", "シェル 1 つのみ、セッションは保持されません")
add("Mature, already on nearly every host", "成熟，几乎每台主机上都有", "成熟しており、ほぼすべてのホストに導入済み")
add("Built for coding agents — agent status needs no hooks",
    "为编码智能体而生——智能体状态无需 hook", "コーディングエージェント向け——エージェントの状態にフックは不要")

# Host / install banners
add("Install herdr", "安装 herdr", "herdr をインストール")
add("herdr not found on this host — plain shell session.",
    "这台主机上找不到 herdr——按普通 shell 会话运行。",
    "このホストに herdr が見つかりません——通常のシェルセッションになります。")
add("herdr isn't installed on this host.", "这台主机上没装 herdr。", "このホストに herdr がインストールされていません。")
add("mosh isn't installed on this host.", "这台主机上没装 mosh。", "このホストに mosh がインストールされていません。")
add("Installs to ~/.local/bin. Moshpit looks there when probing and launching, so you don't need to change PATH.",
    "安装到 ~/.local/bin。Moshpit 探测和启动时都会去那里找，所以你不用改 PATH。",
    "~/.local/bin にインストールされます。Moshpit は検出時も起動時もそこを参照するため、PATH を変更する必要はありません。")
add("Mosh isn't receiving data — your network may be blocking UDP (VPN, proxy, or firewall).",
    "Mosh 收不到数据——你的网络可能在拦 UDP（VPN、代理或防火墙）。",
    "Mosh がデータを受信できていません——ネットワークが UDP を遮断している可能性があります（VPN・プロキシ・ファイアウォール）。")
add("Switch to SSH", "切换到 SSH", "SSH に切り替え")

# SSH keys
add("Select Key", "选择密钥", "キーを選択")
add("SAVED KEYS", "已保存的密钥", "保存済みのキー")
add("No keys yet — generate or import one with ＋", "还没有密钥——用 ＋ 生成或导入", "キーがありません——＋ で生成または読み込みます")
add("Generate SSH Key", "生成 SSH 密钥", "SSH キーを生成")
add("Enter a private key by hand", "手动输入私钥", "秘密鍵を手入力")
add("Import File…", "导入文件…", "ファイルを読み込む…")
add("OR", "或", "または")
add("Copy Public Key", "复制公钥", "公開鍵をコピー")
add("Share Public Key", "分享公钥", "公開鍵を共有")
add("Copy Fingerprint", "复制指纹", "フィンガープリントをコピー")

# Shortcut editor
add("QUICK KEYS", "快捷键预设", "クイックキー")
add("multiplexer prefix", "多路复用器前缀", "マルチプレクサのプレフィックス")
add("Only in a multiplexer", "仅在多路复用器内显示", "マルチプレクサ内のみ")
add("Show only while the session runs tmux or herdr",
    "仅当会话在跑 tmux 或 herdr 时显示", "セッションが tmux または herdr を実行中のときのみ表示します")
add("Tap a preset to fill the trigger — tmux prefixes, control chords, special & navigation keys, F-keys. Then tweak the chip label / color below.",
    "点一个预设即可填入触发键——tmux 前缀、Control 组合、特殊键与方向键、F 键。之后在下面调整键帽文字和颜色。",
    "プリセットをタップするとトリガーが入力されます——tmux プレフィックス、Control コード、特殊キーと移動キー、F キー。その後、下でチップのラベルと色を調整します。")
add("In Key Combo mode the payload is the escape sequence sent to the PTY, honoring the transport transcription rules.",
    "在「组合键」模式下，payload 就是发给 PTY 的转义序列，并遵循传输层的转写规则。",
    "「キーの組み合わせ」モードでは、ペイロードは PTY に送るエスケープシーケンスであり、トランスポートの変換ルールに従います。")

# Themes / icons
add("Untitled", "未命名", "無題")
add("No themes found in that JSON.", "那段 JSON 里没有找到主题。", "その JSON にテーマが見つかりませんでした。")
add("That doesn't look like theme JSON. Paste an exported theme, or an object with \"background\", \"foreground\" and the eight ANSI color names.",
    "这看起来不是主题 JSON。请粘贴导出的主题，或一个带 “background”、“foreground” 和八个 ANSI 颜色名的对象。",
    "テーマの JSON ではないようです。書き出したテーマ、または \"background\"・\"foreground\" と 8 つの ANSI カラー名を持つオブジェクトを貼り付けてください。")
add("Couldn't change the icon: %@", "无法更换图标：%@", "アイコンを変更できませんでした：%@")
add("The icon is separate from the accent color, because iOS only allows switching between icons bundled with the app — a custom accent can't have matching artwork generated for it.",
    "图标和强调色是分开的，因为 iOS 只允许在随 App 打包的图标之间切换——自定义强调色没法自动生成配套图稿。",
    "アイコンとアクセントカラーは別の設定です。iOS はアプリに同梱されたアイコン間の切り替えしか許可しないため、カスタムアクセントに合わせた画像を生成することはできません。")

# Connection / session failures
add("Another client is using this pane — retrying shortly",
    "另一个客户端正在使用这个窗格——稍后重试", "別のクライアントがこのペインを使用中です——しばらくして再試行します")
add("Couldn't reach the host", "无法连到主机", "ホストに到達できませんでした")
add("Creating the worktree failed", "创建工作树失败", "ワークツリーの作成に失敗しました")
add("Removing the worktree failed", "删除工作树失败", "ワークツリーの削除に失敗しました")
add("The worktree was created but has no pane", "工作树已创建，但没有窗格", "ワークツリーは作成されましたが、ペインがありません")
add("Couldn't read the saved credential. After re-installing or re-signing the app (e.g. via SideStore), open Edit and re-enter your password / re-select your key.",
    "读不出已保存的凭据。重装或重签名 App（例如通过 SideStore）之后，请打开「编辑」重新输入密码 / 重新选择密钥。",
    "保存された認証情報を読み取れませんでした。アプリを再インストールまたは再署名した後（SideStore 経由など）は、「編集」を開いてパスワードを再入力するか、キーを選び直してください。")
add("The connection failed for a reason Moshpit didn't recognise. Check the host, port and that the server is reachable, then try again.",
    "连接失败，原因 Moshpit 没能识别。请检查主机、端口以及服务器是否可达，然后重试。",
    "Moshpit が識別できない理由で接続に失敗しました。ホスト・ポート・サーバーに到達できるかを確認して、もう一度お試しください。")
add("The server closed the connection while setting up SSH. Check that the port really is an SSH server, and that a firewall or proxy isn't cutting the connection.",
    "服务器在建立 SSH 的过程中关闭了连接。请确认该端口确实是 SSH 服务，并且没有防火墙或代理在切断连接。",
    "SSH の確立中にサーバーが接続を閉じました。そのポートが本当に SSH サーバーであること、ファイアウォールやプロキシが接続を切断していないことを確認してください。")

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
        # Keep only this checkout's own sources — the same DerivedData holds
        # stringsdata for SwiftTerm, Citadel and every other package, whose
        # keys are not ours to translate. Matched against the repo root rather
        # than a hardcoded folder name, so renaming the checkout doesn't turn
        # this into a silent "no .stringsdata found".
        prefix = root + os.sep
        source = data.get("source", "")
        if not source.startswith(prefix):
            continue
        for entries in (data.get("tables") or {}).values():
            for item in entries:
                if item.get("key"):
                    needed[item["key"]] = source[len(prefix):]
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
