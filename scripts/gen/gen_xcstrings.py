#!/usr/bin/env python3
"""Generate the four strings catalogs: Moshpit/Resources/Localizable.xcstrings (the app),
Extensions/MoshpitIsland/Localizable.xcstrings (the widget),
Extensions/MoshpitPush/Localizable.xcstrings (the notification service extension) and
Extensions/MoshpitShare/Localizable.xcstrings (the share extension).

Source language: en (the key IS the English value).
Each entry carries complete zh-Hans + ja translations inline; zh-Hant, ko, de, es,
fr and pt-BR live in scripts/gen/locales/<locale>.json (see STYLE.md there).
Moshpit/Resources/InfoPlist.xcstrings (permission texts) is written too.
SAME = identical value in every language (brand names, protocol words, glyphs).
"""
import json, os, re, sys

SAME = object()

# key -> (zh-Hans, ja) | SAME | {"plural": {"en_one":..., "en_other":..., "zh":..., "ja":...}}
S = {}

def add(key, zh=SAME, ja=None):
    S[key] = SAME if zh is SAME else (zh, ja)
def plural(key, en_one, zh, ja):
    S[key] = {"plural": True, "en_one": en_one, "zh": zh, "ja": ja}

def plural_spec(key, en_one, zh, ja, en_other=None):
    """A plural entry for any table. `en_other` is for keys whose English
    'other' form differs from the key itself ("%lld needs you" → "need you")."""
    spec = {"plural": True, "en_one": en_one, "zh": zh, "ja": ja}
    if en_other:
        spec["en_other"] = en_other
    return spec

# ---------- Brand / protocol / glyphs (identical everywhere) ----------
for k in ["Moshpit", "MOSH", "JSON"]:
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
add("Disconnect", "断开", "切断")
add("Settings", "设置", "設定")

# ---------- Home (HomeView) ----------
add("No connections yet", "还没有连接", "接続はまだありません")
add("Delete %@?", "删除 %@？", "%@を削除しますか？")
add("connection", "连接", "接続")
add("Removes the saved server and its stored credentials from this device.",
    "这台服务器和它保存的登录凭据都会从本机删除。",
    "保存済みのサーバーと、このデバイスに保存された認証情報を削除します。")
# The NSH Home's refresh button (SwiftUI's read the symbol's name).
# The NSH Home's sidebar button on iPad (UIKit's own is the system's).
add("Hide Sidebar", "隐藏边栏", "サイドバーを非表示")
add("Couldn't delete the connection", "没能删除这个连接", "接続を削除できませんでした")
add("Its push pairing couldn't be removed from this phone, so the connection was kept. The diagnostics log has the reason.",
    "这台手机上它的推送配对信息没能删掉，所以连接也没删。原因在诊断日志里。",
    "この iPhone からプッシュ通知のペアリング情報を削除できなかったため、接続も削除していません。理由は診断ログにあります。")
plural("%lld panes", "%lld pane", "%lld 个窗格", "%lld個のペイン")

# ---------- Home (R6, UI-DESIGN §6.1) ----------
add("Online", "在线", "オンライン")
add("Offline", "离线", "オフライン")
plural("%lld waiting", "%lld waiting", "%lld 个在等你", "%lld 件があなた待ち")
add("Edit…", "编辑…", "編集…")
add("Delete…", "删除…", "削除…")
add("Lost just now", "刚刚断开", "たった今切断")
plural("Lost %lld minutes ago", "Lost %lld minute ago", "%lld 分钟前断开", "%lld 分前に切断")
plural("Lost %lld hours ago", "Lost %lld hour ago", "%lld 小时前断开", "%lld 時間前に切断")
plural("Lost %lld days ago", "Lost %lld day ago", "%lld 天前断开", "%lld 日前に切断")
add("Connecting to tmux…", "正在连接 tmux…", "tmux に接続中…")
add("Connecting to herdr…", "正在连接 herdr…", "herdr に接続中…")
add("Retry connecting tmux", "重试连接 tmux", "tmux への接続を再試行")
add("tmux isn't answering", "tmux 没有回应", "tmux が応答しません")
add("Needs you", "在等你", "あなた待ち")
add("Nothing needs you", "没有在等你的", "あなた待ちはありません")
add("%lld working", "%lld 个工作中", "%lld 件作業中")
plural("%lld done", "%lld done", "%lld 个已完成", "%lld 件完了")
plural("%lld idle", "%lld idle", "%lld 个空闲", "%lld 件待機中")
plural("%lld connections online", "%lld connection online", "%lld 个连接在线", "%lld 件の接続がオンライン")
add("Add a machine you can reach over SSH. Agents running there show up here.", "添加一台能 SSH 的机器，上面跑的智能体会出现在这里。", "SSH で接続できるマシンを追加すると、そこで動いているエージェントがここに表示されます。")
plural("%lld minutes", "%lld minute", "%lld 分钟", "%lld 分")
plural("%lld hours", "%lld hour", "%lld 小时", "%lld 時間")
plural("%lld more working", "%lld more working", "另外 %lld 个工作中", "ほかに %lld 件作業中")
add("All sessions", "全部会话", "すべてのセッション")
add("All workspaces", "全部工作区", "すべてのワークスペース")

add("Sign-in failed", "登录失败", "サインインできませんでした")
add("Host key not confirmed", "主机密钥未确认", "ホストキーが未確認です")
add("The host's key changed", "主机密钥变了", "ホストキーが変わりました")
add("Timed out", "连接超时", "タイムアウトしました")
add("The proxy didn't connect", "代理连不上", "プロキシに接続できません")

# ---------- Add Connection ----------
add("CONNECTION", "连接", "接続")
add("Name", "名称", "名前")
add("Host", "主机", "ホスト")
add("Port", "端口", "ポート")
add("Password", "密码", "パスワード")
# The ui layer's secret field (ui/src/ui/field.rs): a password or key the
# connection keeps, never shown, and what screen readers say for it.
add("Saved", "已保存", "保存済み")
add("Type to replace", "输入即替换", "入力して置き換え")
add("Replaces saved", "替换已保存的", "保存済みを置き換え")
add("%@, saved, type to replace", "%@，已保存，输入可替换", "%@、保存済み、入力で置き換え")
add("%@, replaces saved", "%@，替换已保存的", "%@、保存済みを置き換え")
# A field's name, then its error, as screen readers say them.
add("%@, %@", "%@，%@", "%@、%@")
# The redesign's theme, accent and folder pages (R13/R14, ui/src/theme_gallery.rs,
# accent.rs, accent_editor.rs, theme_editor.rs, terminal/folder.rs).
add("Built-in", "内置", "内蔵")
add("Yours", "自己做的", "自分で作成")
add("Long-press a theme to duplicate, edit or delete it.", "长按主题可以复制、编辑或删除。", "テーマを長押しすると、複製・編集・削除できます。")
add("New…", "新建…", "新規…")
add("Import…", "导入…", "読み込む…")
add("Long-press an accent to edit or delete it.", "长按强调色可以编辑或删除。", "アクセントを長押しすると、編集・削除できます。")
add("ANSI colors", "ANSI 颜色", "ANSIカラー")
add("Bright colors", "亮色", "明るい色")
add("Bright colors follow the eight above. Many tools draw dim text in bright black, so keep it apart from black.", "亮色默认跟着上面八种走。很多工具用亮黑写暗淡的字，别把它调得和黑色一样。", "明るい色は上の8色に合わせて決まります。暗い文字に明るい黒を使うツールが多いので、黒とは区別しておきましょう。")
add("As text in the light appearance this color is too pale, so the app darkens it there.", "这个颜色在浅色外观下做文字会偏淡，系统会自动调深。", "この色はライトモードの文字には淡すぎるため、アプリが自動で濃くします。")
add("Dark", "深色", "ダーク")
add("Light", "浅色", "ライト")
add("Photos…", "照片…", "写真…")
add("Files…", "文件…", "ファイル…")
add("New folder…", "新建文件夹…", "新規フォルダ…")
add("Couldn't open this folder", "打不开这个文件夹", "このフォルダを開けません")
add("This folder is empty", "这个文件夹是空的", "このフォルダは空です")
add("Upload…", "上传…", "アップロード…")
# Connection form (R9, §6.5)
add("Add connection", "添加连接", "接続を追加")
add("Edit connection", "编辑连接", "接続を編集")
add("User", "用户", "ユーザー")
add("SIGN IN", "登录", "ログイン")
add("Sign in with", "登录方式", "ログイン方法")
add("SSH key", "SSH 密钥", "SSH キー")
add("Use mosh", "使用 mosh", "mosh を使う")
add("Survives network changes; needs UDP.", "换网络不断线；需要 UDP。", "ネットワークが変わっても切れません。UDP が必要です。")
add("UDP ports are set in Settings, for every connection.", "UDP 端口在「设置」里统一设。", "UDP ポートは「設定」ですべての接続に共通で設定します。")
add("SOCKS5")
add("Proxies SSH only; mosh's UDP goes direct.", "只代理 SSH；mosh 的 UDP 直连。", "プロキシを通るのは SSH だけです。mosh の UDP は直接つながります。")
add("tmux path", "tmux 路径", "tmux のパス")
add("herdr path", "herdr 路径", "herdr のパス")
add("If the host doesn't have the multiplexer you pick, Moshpit says so and opens a plain shell; it never switches to the other.", "主机上没有选中的终端复用器时，Moshpit 会说明并打开普通 shell，不会换用另一个。", "選んだマルチプレクサがホストにない場合、Moshpit はそう伝えて通常のシェルを開きます。もう一方に切り替えることはありません。")
add("Port must be between 1 and 65535.", "端口要在 1 到 65535 之间。", "ポートは 1〜65535 の範囲で指定してください。")
add("Delete connection…", "删除连接…", "接続を削除…")
add("Couldn't save the password.", "没能保存密码。", "パスワードを保存できませんでした。")
add("Couldn't save the key.", "没能保存密钥。", "キーを保存できませんでした。")
add("Paste PEM", "粘贴 PEM", "PEM を貼り付け")
add("Paste a PEM instead…", "直接粘贴 PEM…", "代わりに PEM を貼り付け…")
add("Private Key (PEM)", "私钥（PEM）", "秘密鍵（PEM）")
add("UDP Port Range", "UDP 端口范围", "UDP ポート範囲")
add("Predict Mode", "预测模式", "予測モード")
add("ADVANCED", "高级", "詳細設定")
add("Add Connection", "添加连接", "接続を追加")
add("Edit Connection", "编辑连接", "接続を編集")
add("Could not save", "无法保存", "保存できませんでした")
# Saving a connection whose sign-in method changed with no new secret: its
# own item holds the old method's (ConnectionCredential.Refusal).
add("To change how this connection signs in, enter the new password or key.",
    "要换登录方式，得填上新的密码或密钥。",
    "サインイン方法を変えるには、新しいパスワードまたは鍵を入力してください。")
add("This connection was deleted while you were editing it.", "编辑期间这个连接已经被删除了。", "編集中にこの接続は削除されました。")
add("As in Settings", "跟随设置", "設定に従う")
add("Same user, host and port as “%@”.", "和「%@」的用户、主机、端口都相同。", "「%@」とユーザー、ホスト、ポートが同じです。")
add("“%@” is already another connection's name.", "已经有别的连接叫「%@」了。", "「%@」という名前の接続がすでにあります。")

# ---------- Settings ----------
add("DISPLAY", "显示", "表示")
add("Font", "字体", "フォント")
add("Theme", "主题", "テーマ")
add("CURSOR", "光标", "カーソル")
add("Shape", "形状", "形状")
add("Block", "方块", "ブロック")
add("Bar", "竖线", "バー")
add("Underline", "下划线", "下線")
add("Color", "颜色", "カラー")
add("Blink", "闪烁", "点滅")
# mosh's predictive echo modes, the Predictive Echo menu's choices.
add("Adaptive", "自适应", "自動調整")
add("Always", "始终", "常に")
add("Never", "从不", "しない")
add("Experimental", "实验性", "実験的")
add("Shortcuts", "快捷键", "ショートカット")
add("SSH Keys", "SSH 密钥", "SSH キー")
add("Notifications", "通知", "通知")
add("Live Activity", "实时活动", "ライブアクティビティ")
add("How notifications work", "通知是怎么工作的", "通知のしくみ")
add("VOICE INPUT", "语音输入", "音声入力")

# ---------- Voice input · engine + model (Whisper) ----------
add("Recognition", "识别引擎", "認識エンジン")
add("Apple (built in)", "Apple（内置）", "Apple（内蔵）")
add("Local model", "本机模型", "ローカルモデル")
add("Best for Chinese with English terms in it. Listens for the file names and branches on screen.",
    "中英混着说最准，还会留意屏幕上的文件名和分支名。",
    "中国語に英語の用語が混ざる話し方に最適。画面に出ているファイル名やブランチ名も手がかりにします。")
add("The server answered %lld.", "服务器返回了 %lld。", "サーバーの応答は %lld でした。")
add("%@ arrived damaged. Try again.", "%@ 下载下来是坏的，再试一次。", "%@ が壊れた状態で届きました。もう一度お試しください。")
add("No download. One language per session.", "无需下载。每次会话只识别一种语言。", "ダウンロード不要。1 セッションに 1 言語。")
add("Downloads a model. Around 100 languages, and copes with a sentence that mixes two.",
    "需下载模型。覆盖约 100 种语言，一句话里混用两种也能听准。",
    "モデルのダウンロードが必要。約 100 言語に対応し、1 つの文に 2 言語が混在しても処理できます。")

add("Model", "模型", "モデル")
# Variant names stay verbatim: they're the identifiers users match against
# Hugging Face and Whisper's own docs, so translating them would break lookup.
for k in ["Tiny", "Base", "Small", "Large v3 Turbo"]:
    add(k, SAME)
add("Most accurate, especially on mixed-language speech. Needs a recent chip.",
    "准确度最高，尤其是多语种混说。需要较新的芯片。",
    "最も高精度。特に複数言語が混在する音声に強い。新しめのチップが必要です。")
add("Good balance of accuracy and speed.", "准确度与速度较均衡。", "精度と速度のバランスが良い。")
add("Fast and light. Noticeably weaker outside English.",
    "快、占用小。英语以外的语言明显差一些。", "高速・軽量。英語以外は精度が明らかに落ちます。")
add("Fastest, lowest accuracy. For older devices.",
    "最快，准确度最低。适合较旧的设备。", "最速・最低精度。古い端末向け。")
add("Download", "下载", "ダウンロード")
add("Resume", "继续", "再開")
add("Pause", "暂停", "一時停止")
add("Downloading %@", "正在下载 %@", "%@ をダウンロード中")
add("Remove this model?", "删除这个模型？", "このモデルを削除しますか？")
add("Remove", "删除", "削除")
add("Keep", "保留", "残す")
add("%@ frees %@. You can download it again later.",
    "删除 %1$@ 可释放 %2$@。以后可以重新下载。",
    "%1$@ を削除すると %2$@ を解放できます。後で再ダウンロードできます。")
add("Models on this device", "本机已下载", "この端末のモデル")

# ---------- Voice input · language picker ----------
add("Voice Language", "语音语言", "音声入力の言語")
add("Automatic", "自动", "自動")
add("Auto-detect", "自动检测", "自動検出")
add("Whisper decides from what it hears", "让 Whisper 自己听出是哪种语言", "Whisper が聞こえた内容から判断します")

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
add("Dictation isn't available for %@ on this device.", "这台设备不支持 %@ 听写。", "この端末では %@ の音声入力を利用できません。")
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
# Offline host-setup sheet
add("Host setup is automatic. Connecting to a host installs and repairs everything it needs — scripts, hooks registration, push pairing — and asks before its first install. Connect to a host to inspect or remove its setup here.",
    "主机这边不用手动配置。每次连接，Moshpit 都会把需要的东西自动装好、修好：脚本、状态脚本的注册、推送配对，第一次安装前会先问你。连上主机后，可以在这里查看或移除。",
    "ホスト設定は自動です。ホストに接続すると必要なもの——スクリプト、フック登録、プッシュペアリング——を自動でインストール・修復し、初回インストール前には確認します。接続するとここで確認・削除できます。")
# Notification info sheet
add("Agents stamp their state", "智能体自报状态", "エージェントが状態を報告")
add("Coding agents (Claude Code, Codex, …) report working / needs-you / done through hooks Moshpit installs on your host — precise states, not guesses. The terminal bell still works as a fallback for everything else.",
    "Claude Code、Codex 这类编程智能体，靠 Moshpit 装在主机上的状态脚本自己报状态：工作中、在等你、已完成。状态是准的，不靠猜。其他程序还是靠终端响铃兜底。",
    "コーディングエージェント（Claude Code、Codex など）は Moshpit がホストにインストールするフックで「作業中 / あなた待ち / 完了」を報告します——推測ではなく正確な状態です。それ以外はターミナルベルがフォールバックとして機能します。")
add("While a session is attached, the Dynamic Island shows whether the agent is working, idle, or waiting on you. Tapping it deep-links straight back to that pane.",
    "连着会话时，灵动岛会显示智能体是在工作、闲着，还是在等你。点一下直接回到那个窗格。",
    "セッションのアタッチ中、Dynamic Island はエージェントが作業中・アイドル・あなた待ちのいずれかを表示します。タップするとそのペインへ直接ジャンプします。")
add("Push, sealed end-to-end", "端到端加密的推送", "プッシュ、エンドツーエンド暗号化")
add("When the app isn’t running, your host sends the alert through Moshpit’s push relay. It is encrypted on your host with a key only this device holds — the relay and Apple carry ciphertext and can read none of it. Delivered even from a locked phone.",
    "App 没在运行时，你的主机经 Moshpit 的推送服务把通知发过来。内容在主机上就用只有这台手机才有的密钥加密，推送服务和 Apple 经手的都是密文，一个字也读不到。锁屏时也能收到。",
    "アプリが起動していないとき、通知はホストから Moshpit のプッシュリレー経由で届きます。内容はこのデバイスだけが持つ鍵でホスト上で暗号化され、リレーも Apple も暗号文を運ぶだけで読めません。ロック中の iPhone にも届きます。")
add("Quiet by design", "尽量不打扰", "静けさを前提に")
add("A question must stand for 30 seconds before any phone hears about it — answered at your desk means never announced. All waiting agents share one summary card; only the first rings. A finished turn shows the prompt it answered and only chimes if it ran three minutes or more. Parked agents stay silent.",
    "智能体的提问要等 30 秒还没人答，手机才会收到通知。在电脑前顺手答掉，手机就不会响。所有等你的智能体合成一张摘要卡，只有第一条会响铃。完成通知会带上它刚做完的那条提示词，跑满三分钟的才有提示音。闲着的智能体不发通知。",
    "問いかけは 30 秒間続いて初めて通知されます——デスクですぐ答えれば鳴りません。待機中のエージェントは 1 枚のサマリーカードを共有し、鳴るのは最初の 1 回だけ。完了通知には答えたプロンプトが載り、完了音は 3 分以上かかったターンのみ。放置中のエージェントは静かなままです。")
# UDP editor
add("UDP PORT RANGE", "UDP 端口范围", "UDP ポート範囲")
add("mosh binds one UDP port in this range per session. Open it outbound from your iPhone and inbound on the server (default 60000–61000).",
    "每个 mosh 会话会在这段范围里占一个 UDP 端口。服务器要放开入站，iPhone 这边要放开出站（默认 60000–61000）。",
    "mosh はセッションごとにこの範囲の UDP ポートを 1 つバインドします。iPhone 側は送信方向、サーバー側は受信方向で開放してください（デフォルト 60000–61000）。")
add("From", "起始", "開始")
add("To", "结束", "終了")
add("Enter numeric ports.", "请输入数字端口。", "数値のポートを入力してください。")
add("Ports must be 1–65535.", "端口必须在 1–65535 之间。", "ポートは 1–65535 の範囲で指定してください。")
add("From must be ≤ To.", "起始值必须 ≤ 结束值。", "開始は終了以下にしてください。")

# ---------- SSH Keys ----------
add("THIS DEVICE · SECURE ENCLAVE", "本机 · SECURE ENCLAVE", "このデバイス · SECURE ENCLAVE")
add("Hardware-backed keys cannot be exported; every signature triggers Face ID.",
    "密钥存在安全芯片里，导不出来，每次签名都要过 Face ID。",
    "ハードウェア保護されたキーは書き出せません。署名のたびに Face ID が起動します。")
# Android's words where the shared screens say iOS's (B258): the Android
# Keystore for the Secure Enclave and the Keychain, this device for the
# iPhone, an email app for Mail.
add("THIS DEVICE · ANDROID KEYSTORE", "本机 · Android 密钥库", "このデバイス · ANDROID KEYSTORE")
add("Keys in the Android Keystore cannot be exported; every signature is made inside it.",
    "存在 Android 密钥库里的密钥导不出来，每次签名都在密钥库里完成。",
    "Android Keystore のキーは書き出せません。署名はすべて Keystore の中で行われます。")
add("Generates the key pair on device. The private key can optionally be held in the Android Keystore — used only for signing, never leaving it.",
    "本地生成密钥对。私钥可选交给 Android 密钥库保管，只在签名时调用，不出密钥库。",
    "デバイス上で鍵ペアを生成します。秘密鍵は Android Keystore に保管でき、署名時のみ使用され、Keystore の外には出ません。")
add("Android Keystore keys are limited to ECDSA-P256; the algorithm adjusts automatically when the Keystore is on.",
    "Android 密钥库只支持 ECDSA-P256，打开后算法会自动切换。",
    "Android Keystore のキーは ECDSA-P256 に限定されます。Keystore を有効にするとアルゴリズムは自動調整されます。")
add("Store in Android Keystore", "存入 Android 密钥库", "Android Keystore に保存")
add("This key is protected by a passphrase. It's used once, to unlock the key here; from then on the Android Keystore protects it.",
    "这把密钥设了口令。口令只在导入时用一次，解开后密钥由 Android 密钥库保护。",
    "この鍵はパスフレーズで保護されています。パスフレーズは読み込み時に一度だけ使い、以降の鍵は Android Keystore が保護します。")
add("IMPORTED", "已导入", "インポート済み")
add("No imported keys", "没有已导入的密钥", "インポートされたキーはありません")
# Keys whose private half isn't on this device: a phone restored from a backup (B781)
add("NOT ON THIS IPHONE",
    "不在这台 iPhone 上",
    "この iPhone にはありません")
add("This key isn't on this iPhone",
    "这台 iPhone 上没有这把密钥",
    "このキーはこの iPhone にありません")
add("Keys stay on the device they were imported on. Import it again or delete this entry.",
    "密钥只留在导入它的那台设备上。可以重新导入，或删掉这条记录。",
    "キーはインポートしたデバイスにだけ残ります。もう一度インポートするか、この項目を削除してください。")
add("This key was made in another device's Secure Enclave and can't be moved. Delete it and make a new one.",
    "这把密钥生成在另一台设备的安全芯片里，带不过来。删掉它，再新建一把。",
    "このキーは別のデバイスの Secure Enclave で作られたため、移せません。削除して新しいキーを作ってください。")
add("Import Again…",
    "重新导入…",
    "再インポート…")
add("Import Again",
    "重新导入",
    "再インポート")
add("That's a different key from the one this entry was made with. Import that key, or delete this entry.",
    "这不是当初这条记录用的那把密钥。请导入那把，或者删掉这条记录。",
    "この項目を作ったときのキーとは別のキーです。そのキーをインポートするか、この項目を削除してください。")
# SSH Keys on the ui layer (R10): an empty group's button, a key's use, a copy's toast.
add("No device key yet", "还没有设备密钥", "デバイスキーはまだありません")
add("Generate key", "生成密钥", "キーを生成")
add("Import key", "导入密钥", "キーをインポート")
add("Used for %@", "用于 %@", "%@ で使用")
add("Public key copied", "已复制公钥", "公開鍵をコピーしました")
add("Fingerprint copied", "已复制指纹", "フィンガープリントをコピーしました")
add("No keys yet", "还没有密钥", "キーはまだありません")
# Add Key on the ui layer (R10): its fields' visible labels.
add("Private key", "私钥", "秘密鍵")
add("Public key", "公钥", "公開鍵")
add("Import Public Key File…", "导入公钥文件…", "公開鍵ファイルを読み込む…")
add("Fingerprint", "指纹", "フィンガープリント")
# Shortcuts on the ui layer (R11): its groups' titles, a full bar, a delete's question.
add("In the bar · %lld of %lld", "在栏上 · %lld/%lld", "バー上 · %lld/%lld")
add("Available", "可以加的", "追加できるもの")
add("The bar holds up to %lld.", "栏上最多 %lld 个。", "バーに置けるのは %lld 個までです。")
add("It comes off the toolbar and out of your shortcuts.", "它会从快捷键栏和你的快捷键里删掉。", "ツールバーとあなたのショートカットから削除されます。")
# Add Shortcut on the ui layer (R11): the modifiers' chips, the fields' labels, the colors' names.
add("⌃ Control", "⌃ Control 键", "⌃ Control キー")
add("⌥ Option", "⌥ Option 键", "⌥ Option キー")
add("⇧ Shift", "⇧ Shift 键", "⇧ Shift キー")
add("⌘ Command", "⌘ Command 键", "⌘ Command キー")
add("Sends", "发送", "送信内容")
add("Payload", "内容", "内容")
add("At most 6 characters.", "最多 6 个字符。", "6 文字まで。")
add("Gray", "灰色", "グレー")
add("Amber", "琥珀色", "アンバー")
add("Delete Key", "删除密钥", "キーを削除")
add("unused since %@", "自 %@ 起未使用", "%@ 以降未使用")
add("Its private key is erased from this device for good; Moshpit keeps no other copy. Servers that only trust this key will need another one.",
    "它的私钥会从这台设备上彻底删除，Moshpit 不会另存副本。只认这把密钥的服务器，之后得换一把。",
    "秘密鍵はこのデバイスから完全に削除され、Moshpit には控えも残りません。このキーしか登録していないサーバーには、別のキーが必要になります。")
add("Key in use",
    "密钥还在用",
    "使用中のキー")
add("Couldn't delete the key",
    "没能删除这把密钥",
    "キーを削除できませんでした")
add("Recovered key %@",
    "找回的密钥 %@",
    "復元したキー %@")
add("Connections using it: %@. Choose another key for them first, then delete this one.",
    "还在用它的连接：%@。先给它们换一把密钥，再删这把。",
    "このキーを使っている接続：%@。先にそれらの接続で別のキーを選んでから削除してください。")
add("Moshpit can't tell yet which connections use this key, so it was kept. Try again in a moment.",
    "Moshpit 暂时还看不出哪些连接在用这把密钥，所以先留着。稍后再试。",
    "このキーを使っている接続をまだ確認できないため、キーは残しました。少し待ってからもう一度お試しください。")
add("Its private key couldn't be removed from the keychain, so the key was kept. The diagnostics log has the reason.",
    "它的私钥没能从钥匙串里删掉，所以这把密钥也留着。原因在诊断日志里。",
    "秘密鍵をキーチェーンから削除できなかったため、キーは残しました。理由は診断ログにあります。")
# Add Key
add("Add Key", "添加密钥", "キーを追加")
add("METHOD", "方式", "方式")
add("Generates the key pair on device. The private key can optionally be held in the Secure Enclave — used only for signing, never leaving the chip.",
    "本地生成密钥对。私钥可选 Secure Enclave 托管，仅签名时调用，不出芯片。",
    "デバイス上で鍵ペアを生成します。秘密鍵は Secure Enclave に保管でき、署名時のみ使用され、チップの外には出ません。")
add("Generate", "生成", "生成")
add("Import", "导入", "インポート")
add("KEY DETAILS", "密钥详情", "キーの詳細")
add("ED25519 recommended: 32-byte keys, fast signatures, no practical break to date.",
    "推荐 ED25519：32 字节密钥，签名快，目前未被实战攻破。",
    "ED25519 推奨：32 バイト鍵で署名が高速、現時点で実用的な攻撃は知られていません。")
add("Algorithm", "算法", "アルゴリズム")
add("Comment", "备注", "コメント")
add("PROTECTION", "保护", "保護")
add("Secure Enclave keys are limited to ECDSA-P256; the algorithm adjusts automatically when SE is enabled.",
    "Secure Enclave 只支持 ECDSA-P256，开启后算法会自动切换。",
    "Secure Enclave のキーは ECDSA-P256 に限定されます。SE を有効にするとアルゴリズムは自動調整されます。")
add("Require Face ID", "需要 Face ID", "Face ID を要求")
add("Face ID confirmation before every signature", "每次签名前需 Face ID 确认", "署名のたびに Face ID で確認")
add("Store in Secure Enclave", "存入 Secure Enclave", "Secure Enclave に保存")
add("Hardware-backed · private key cannot be exported", "硬件托管 · 私钥不可导出", "ハードウェア保護 · 秘密鍵は書き出し不可")
add("PRIVATE KEY", "私钥", "秘密鍵")
add("Paste an OpenSSH or PEM private key; the public key and fingerprint come from it, so the public key line is optional.",
    "粘贴 OpenSSH 或 PEM 格式的私钥。公钥和指纹都从私钥算出来，公钥那一行可以不填。",
    "OpenSSH または PEM 形式の秘密鍵を貼り付けます。公開鍵とフィンガープリントは秘密鍵から求めるので、公開鍵行は省略できます。")
add("This key is protected by a passphrase. It's used once, to unlock the key here; from then on the Keychain protects it, behind Face ID when required.",
    "这把密钥设了口令。口令只在导入时用一次，解开后密钥存进钥匙串，开了「需要 Face ID」就得先过 Face ID。",
    "この鍵はパスフレーズで保護されています。パスフレーズは読み込み時に一度だけ使い、以降の鍵はキーチェーンが保護します（Face ID を要求している場合は Face ID も）。")
add("Passphrase", "口令", "パスフレーズ")
# SSHKeyImportError (SSHPrivateKeyFile.swift)
add("This isn't a private key Moshpit can read. Paste an OpenSSH or PEM private key, from its BEGIN line to its END line.",
    "Moshpit 认不出这是私钥。请粘贴 OpenSSH 或 PEM 格式的私钥，从 BEGIN 那一行一直到 END 那一行。",
    "Moshpit が読める秘密鍵ではありません。OpenSSH または PEM 形式の秘密鍵を、BEGIN の行から END の行まで貼り付けてください。")
add("This private key is damaged or incomplete. Copy it again, from its BEGIN line to its END line.",
    "这个私钥不完整或已损坏。从 BEGIN 那一行到 END 那一行重新复制一遍。",
    "この秘密鍵は壊れているか、途中で切れています。BEGIN の行から END の行までもう一度コピーしてください。")
add("This key is protected by a passphrase. Enter it to import the key.",
    "这把密钥设了口令，输入口令才能导入。",
    "この鍵はパスフレーズで保護されています。読み込むにはパスフレーズを入力してください。")
add("That passphrase doesn't unlock this key.", "口令不对，解不开这把密钥。", "このパスフレーズでは鍵を開けません。")
add("Security keys (sk keys), such as a YubiKey's, aren't supported yet.",
    "暂不支持 YubiKey 这类安全密钥（sk 密钥）。",
    "YubiKey などのセキュリティキー（sk キー）にはまだ対応していません。")
add("Moshpit can't sign in with %@ keys. Ed25519, RSA and ECDSA keys work.",
    "Moshpit 不能用 %@ 密钥登录。Ed25519、RSA 和 ECDSA 密钥都可以。",
    "Moshpit は %@ 鍵ではサインインできません。Ed25519、RSA、ECDSA の鍵が使えます。")
add("This key is encrypted with %@, which Moshpit can't unlock. Re-encrypt it with “ssh-keygen -p” and import it again.",
    "这把密钥用 %@ 加密，Moshpit 解不开。用 ssh-keygen -p 重新加密后再导入。",
    "この鍵は %@ で暗号化されていて、Moshpit では開けません。「ssh-keygen -p」で暗号化し直してから、もう一度読み込んでください。")
add("Public key line (optional)", "公钥行（可选）", "公開鍵行（任意）")
# A key stored as it was pasted, met at connect (StoredKey, SSHService)
add("This key is locked with a passphrase. Connect again and enter it — Moshpit asks for it only once.",
    "这把密钥设了口令。重新连接时输入一次就行，之后不会再问。",
    "この鍵はパスフレーズで保護されています。もう一度接続して入力してください。尋ねるのはこの一度だけです。")
# Face ID or the passphrase card cancelled (B392)
add("Signing in was cancelled, so Moshpit didn't connect. Connect again to try once more.",
    "登录已取消，没有连接。再连一次就能重试。",
    "サインインがキャンセルされたため、接続しませんでした。もう一度接続すれば再試行できます。")
add("The private key saved for this connection is damaged, or isn't one Moshpit can read. Import or paste the key again.",
    "这个连接保存的私钥已损坏，或者不是 Moshpit 能读的格式。请重新导入或粘贴这把密钥。",
    "この接続に保存された秘密鍵は壊れているか、Moshpit が読める形式ではありません。鍵をもう一度読み込むか、貼り付けてください。")
add("Key Passphrase", "密钥口令", "鍵のパスフレーズ")
add("The key for %@ is locked with a passphrase. Enter it once: from then on the Keychain on this device protects the key, as it does every key you import.",
    "连接 %@ 用的密钥设了口令。输入一次就好，之后这把密钥由本机钥匙串保护，和你导入的其他密钥一样。",
    "%@ に使う鍵はパスフレーズで保護されています。一度入力すれば、以降はほかの読み込んだ鍵と同じように、この端末のキーチェーンが鍵を保護します。")
add("Unlock", "解锁", "ロック解除")
# The server's own question at sign-in, a one-time code (B563)
add("Sign In to %@", "登录 %@", "%@ にサインイン")
add("Answer", "回答", "回答")
add("Can't sign in with %@ keys yet", "暂时还不能用 %@ 密钥登录", "%@ 鍵ではまだサインインできません")
add("—:—:—:—:—:—:—:—:—:—  (shown after generation)", "—:—:—:—:—:—:—:—:—:—（生成后显示）", "—:—:—:—:—:—:—:—:—:—  （生成後に表示）")
add("Key error", "密钥错误", "キーエラー")

# ---------- Shortcuts ----------
add("No custom shortcuts yet", "还没有自定义快捷键", "カスタムショートカットはまだありません")
add("Saved under AVAILABLE", "已存到「可用」里", "「利用可能」に保存しました")
add("The toolbar is full: it holds %lld shortcuts. Take one off with −, then add this one with ＋.",
    "快捷键栏满了，最多放 %lld 个。先用 − 拿掉一个，再用 ＋ 把这个加上去。",
    "ツールバーがいっぱいです（最大 %lld 個）。− でどれかを外してから、＋ でこれを追加してください。")
add("Add Shortcut", "添加快捷键", "ショートカットを追加")
add("Edit Shortcut", "编辑快捷键", "ショートカットを編集")
add("Live preview of your input below. Saved shortcuts land in the Custom group of the Shortcuts toolbar.",
    "下面怎么填，这里就怎么显示。保存后会出现在快捷键栏的「自定义」分组里。",
    "下の入力をリアルタイムでプレビューします。保存するとショートカットツールバーのカスタムグループに入ります。")
add("TYPE", "类型", "種類")
add("Key Combo: sends modifiers + key. Text: types a string as-is. Command: submits one command with Return.",
    "「按键组合」发送修饰键加一个键。「文本」原样输入一段文字。「命令」输入一条命令并回车。",
    "Key Combo：修飾キー＋キーを送信。Text：文字列をそのまま入力。Command：Return でコマンドを 1 つ送信。")
add("Key Combo", "按键组合", "キーコンボ")
add("Text", "文本", "テキスト")
add("Command", "命令", "コマンド")
add("TRIGGER", "触发键", "トリガー")
add("Tap modifiers to toggle them and enter a single character as the main key; the mobile keyboard triggers the raw scancode automatically.",
    "点修饰键切换开关，主键填一个字符。手机键盘会自动发对应的原始扫描码。",
    "修飾キーをタップして切り替え、メインキーは 1 文字で入力します。モバイルキーボードは生のスキャンコードを自動送信します。")
add("Main key", "主键", "メインキー")
add("ACTION", "动作", "アクション")
add("Payload (text or command)", "内容（文本或命令）", "ペイロード（テキストまたはコマンド）")
add("Append Return", "追加 Return", "Return を追加")
add("Automatically appends ⏎ to submit", "发送后自动补一个 ⏎", "送信時に自動で ⏎ を追加")
add("Repeat on hold", "长按重复", "長押しでリピート")
add("Repeats at 30/s after a 0.4s hold", "长按 0.4 秒后，每秒重复 30 次", "0.4 秒の長押し後、毎秒 30 回リピート")
add("Chip Label is capped at 6 characters; Description appears in the edit list to keep shortcuts recognizable.",
    "标签最多 6 个字符。描述只在编辑列表里显示，方便你认出它。",
    "チップラベルは最大 6 文字。説明は編集リストに表示され、見分けやすくなります。")
add("Chip Label", "标签", "チップラベル")
add("Description", "描述", "説明")
add("SCOPE", "使用范围", "スコープ")
add("Available everywhere when nothing is selected. When bound, it appears only in the toolbar of the selected hosts, saving slots in the 12-slot budget.",
    "不选主机就到处都能用。绑定后只在选中主机的快捷键栏出现，给别的快捷键省出位置（一共 12 个）。",
    "未選択ならどこでも使えます。バインドすると選択ホストのツールバーにのみ表示され、12 スロットの枠を節約できます。")
add("SLOT %lld/%lld", "位置 %lld/%lld", "スロット %lld/%lld")
add("unnamed", "未命名", "名称未設定")
add("%@ · sends %@", "%1$@ · 发送 %2$@", "%1$@ · %2$@ を送信")
add("%@ · types \"%@\"", "%1$@ · 键入 \"%2$@\"", "%1$@ · \"%2$@\" を入力")
add("%@ · runs %@", "%1$@ · 运行 %2$@", "%1$@ · %2$@ を実行")

# ---------- Terminal ----------
add("Connection Error", "连接错误", "接続エラー")
add("Trust New Key", "信任新密钥", "新しいキーを信頼")
add("Moshpit saved two different keys for %@:%@ in an earlier version. The key it sent now is %@. Trust this key?",
    "早先的版本为 %@:%@ 存过两把不同的密钥。它这次发来的是 %@，信任这把吗？",
    "以前のバージョンで %@:%@ に異なる2つのキーが保存されています。今回送られてきたのは %@ です。このキーを信頼しますか？")
add("New Host", "新主机", "新しいホスト")
add("Trust", "信任", "信頼")
add("Connecting…", "正在连接…", "接続中…")
add("Creating…", "创建中…", "作成中…")
add("The session ended: the remote shell exited or mosh-server was stopped.",
    "会话已经结束：远程主机上的 shell 退出了，或者 mosh-server 被停掉了。",
    "セッションは終了しました。リモートのシェルが終了したか、mosh-server が停止されました。")
add("The session ended: the remote shell exited.",
    "会话已经结束：远程主机上的 shell 退出了。",
    "リモートのシェルが終了したため、セッションは終了しました。")
# tmux sheets
add("Windows", "窗口", "ウィンドウ")
add("Sessions", "会话", "セッション")
add("Select Pane", "选择窗格", "ペインを選択")
add("Swipe to switch", "滑动切换", "スワイプで切り替え")
# The terminal's sideways swipe between a session's panes (ui/src/terminal/switcher.rs,
# CoreTerminalView+Swipe.swift): the one-time hint, the dots' spoken position,
# VoiceOver's actions on the terminal.
add("Swipe sideways to switch panes", "左右滑动，切换窗格", "左右にスワイプしてペインを切り替え")
add("Pane %lld of %lld, %@", "第 %lld 个窗格，共 %lld 个，%@", "ペイン %lld/%lld、%@")
add("Next pane", "下一个窗格", "次のペイン")
add("Previous pane", "上一个窗格", "前のペイン")
# The pane overview (ui/src/terminal/overview.rs, the terminal's long-press menus,
# VoiceOver's action and ⌘K's title).
plural("%lld windows", "%lld window", "%lld 个窗口", "%lld 個のウィンドウ")
plural("%lld tabs", "%lld tab", "%lld 个标签页", "%lld 個のタブ")
add("Rename window…", "重命名窗口…", "ウィンドウ名を変更…")
add("Rename tab…", "重命名标签页…", "タブ名を変更…")
add("Kill pane…", "终止窗格…", "ペインを強制終了…")
add("Close pane…", "关闭窗格…", "ペインを閉じる…")
add("Kill window…", "终止窗口…", "ウィンドウを強制終了…")
add("Close tab…", "关闭标签页…", "タブを閉じる…")
add("Kill pane “%1$@” in %2$@? What runs in it stops.",
    "终止 %2$@ 里的窗格「%1$@」？里面正在跑的程序会停掉。",
    "%2$@ のペイン「%1$@」を強制終了しますか？中で動いているものは止まります。")
add("Close pane “%1$@” in %2$@? What runs in it stops.",
    "关闭 %2$@ 里的窗格「%1$@」？里面正在跑的程序会停掉。",
    "%2$@ のペイン「%1$@」を閉じますか？中で動いているものは止まります。")
add("Kill pane", "终止窗格", "ペインを強制終了")
add("Close pane", "关闭窗格", "ペインを閉じる")
add("New window", "新建窗口", "新しいウィンドウ")
add("New tab", "新建标签页", "新しいタブ")
add("Show all panes", "显示全部窗格", "すべてのペインを表示")
add("Show All Panes", "显示全部窗格", "すべてのペインを表示")
add("attached", "已附加", "アタッチ済み")
add("detached", "已分离", "デタッチ済み")

# ---------- Backfill ----------
# Strings that shipped untranslated: they were in the app but never in this
# table, so a zh-Hans or ja user saw English. Found with `--check`, which reads
# the keys the compiler extracted rather than trusting this file to be complete.
# Kept as one block per screen rather than merged into the sections above so the
# backfill stays reviewable as a unit.

# Home (HomeView)
add("%@", SAME)
add("now", "刚刚", "たった今")
add("agent", "智能体", "エージェント")
add("%lld agents need you", "%lld 个智能体在等你", "エージェント %lld 体があなたを待っています")
add("Disconnecting…", "正在断开…", "切断中…")
add("Leave blank for an automatic name.", "留空则自动命名。", "空欄にすると自動で名前が付きます。")
add("Names can't contain line breaks or other control characters.", "名称里不能有换行或其他控制字符。",
    "名前に改行やその他の制御文字は使えません。")
# mosh + tmux: what the person asked of tmux while the control plane was down
# didn't come through, and it is back (B401).
add("Your last tmux action didn't go through: the control connection dropped, and it's back now.",
    "刚才的 tmux 操作没有生效：控制连接断了，已重新连上。",
    "直前の tmux 操作は反映されませんでした。制御用の接続が切れ、つなぎ直しました。")
add("Remove Worktree", "删除工作树", "ワークツリーを削除")
add("Remove the worktree for \"%@\"?", "删除 “%@” 的工作树？", "「%@」のワークツリーを削除しますか？")
add("Deletes the branch checkout under ~/.herdr/worktrees. %@ itself is untouched.",
    "只删除 ~/.herdr/worktrees 下的分支检出，%@ 本身不受影响。",
    "~/.herdr/worktrees 配下のブランチのチェックアウトのみを削除します。%@ 自体はそのままです。")
add("\"%@\" has uncommitted changes", "“%@” 有未提交的改动", "「%@」に未コミットの変更があります")
add("Those changes exist nowhere else. Removing the worktree throws them away.",
    "这些改动只存在这里。删掉工作树，它们就没了。",
    "これらの変更は他のどこにも存在しません。ワークツリーを削除すると失われます。")
add("Delete anyway", "仍然删除", "それでも削除")
add("Keep it", "保留", "残す")
add("Couldn't remove the worktree", "无法删除工作树", "ワークツリーを削除できませんでした")
# A screen whose body failed (B366): the page says so and offers Retry.
add("Couldn't show this page", "这一页没能显示出来", "このページを表示できませんでした")
add("%@ %@ \"%@\"?", SAME)
add("%@ pane %lld?", "%@ 窗格 %lld？", "%@ ペイン %lld？")

# Terminal
add("Control", "Control 键", "Control キー")
add("Control armed", "Control 待命", "Control キー待機中")
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
add("MOSH DIAGNOSTICS", "MOSH 诊断", "MOSH 診断")
add("No datagrams yet.", "还没收到任何数据。", "まだデータグラムを受信していません。")

add("Theme name", "主题名称", "テーマ名")
add("Theme preview", "主题预览", "テーマのプレビュー")
add("These eight are what shells, diffs and TUIs paint with.",
    "shell、diff 和 TUI 用的就是这八种颜色。",
    "シェル・diff・TUI が使うのはこの 8 色です。")
add("Override bright colors", "自定义高亮颜色", "明るいカラーを上書き")
add("Copy as JSON", "复制为 JSON", "JSON としてコピー")
# The editor's color rows (the SwiftUI page passed them as plain strings and
# showed them in English everywhere; the NSH page looks them up).
add("Background", "背景", "背景")
add("Foreground", "前景", "前景")
add("Cursor", "光标", "カーソル")
for key, zh, ja in [("Black", "黑色", "黒"), ("Red", "红色", "赤"), ("Green", "绿色", "緑"),
                    ("Yellow", "黄色", "黄"), ("Blue", "蓝色", "青"), ("Magenta", "品红色", "マゼンタ"),
                    ("Cyan", "青色", "シアン"), ("White", "白色", "白")]:
    add(key, zh, ja)
    add("Bright " + key, "亮" + zh, "明るい" + ja)

# Theme gallery
add("New Theme", "新建主题", "新規テーマ")
add("Add theme", "添加主题", "テーマを追加")
add("Duplicate", "复制", "複製")
add("Duplicate & Edit", "复制并编辑", "複製して編集")
add("Copy JSON", "复制 JSON", "JSON をコピー")
add("Import Theme", "导入主题", "テーマを読み込む")
add("Paste from Clipboard", "从剪贴板粘贴", "クリップボードから貼り付け")
add("No custom themes yet.", "还没有自定义主题。", "カスタムテーマはまだありません。")
add("This theme will be removed. Built-in themes are unaffected.",
    "这个主题会被删除。内置主题不受影响。", "このテーマを削除します。内蔵テーマには影響しません。")
add("Paste a theme exported from Moshpit, or any JSON object with \"background\", \"foreground\" and the eight ANSI color names (\"black\", \"red\", …). Bright colors are optional.",
    "粘贴从 Moshpit 导出的主题，或任何带 “background”、“foreground” 和八个 ANSI 颜色名（“black”、“red” …）的 JSON 对象。高亮颜色可选。",
    "Moshpit から書き出したテーマ、または \"background\"・\"foreground\" と 8 つの ANSI カラー名（\"black\"、\"red\" …）を持つ JSON オブジェクトを貼り付けます。明るいカラーは任意です。")

# Accent gallery
add("New accent", "新建强调色", "新規アクセント")
add("New Accent", "新建强调色", "新規アクセント")
add("Accent name", "强调色名称", "アクセント名")
add("Accent preview", "强调色预览", "アクセントのプレビュー")
add("No custom accents yet.", "还没有自定义强调色。", "カスタムアクセントはまだありません。")
add("The pressed state and the background wash are derived from this one color.",
    "按下态和背景微光都由这一个颜色推导。", "押下時の状態と背景のごく淡い色は、この 1 色から算出されます。")
add("This accent will be removed. Built-in accents are unaffected.",
    "这个强调色会被删除。内置强调色不受影响。", "このアクセントを削除します。内蔵アクセントには影響しません。")
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
add("Looking for repositories…", "正在查找仓库…", "リポジトリを検索中…")
add("None found — no panes in repos, and nothing under ~",
    "没找到——窗格都不在仓库里，主目录下也没有", "見つかりません——リポジトリ内のペインがなく、ホーム直下にもありません")
add("Creates a git worktree on the host, then starts the agent inside it. Your working tree is untouched.",
    "在主机上新开一个 git worktree，让智能体在里面干活。你现在的工作树不会被动。",
    "ホスト上に git ワークツリーを作成し、その中でエージェントを起動します。あなたの作業ツリーはそのままです。")
add("Optional. Sent to the agent once it's running — leave blank to type it yourself.",
    "可选。智能体启动后会发给它——留空则你自己手输。",
    "任意。エージェントの起動後に送信されます——空欄にすると自分で入力できます。")

# Settings
add("Accent", "强调色", "アクセント")
add("App Icon", "应用图标", "アプリアイコン")
add("Language", "语言", "言語")
add("Copy", "复制", "コピー")
# The terminal's edit menu: opens the find bar (the core's view, T3).
add("Find", "查找", "検索")
# Settings footer: the MIIT APP filing number (the %@), shown to satisfy the
# regulator's in-app display rule. The label is the only translatable part.
add("APP filing %@", "APP 备案号 %@", "APP 届出番号 %@")
add("Alert when an agent needs you", "有智能体在等你时提醒", "エージェントがあなたを必要とするとき通知")
add("Alert sound", "提示音", "通知音")
add("Play a sound when the agent needs you", "智能体在等你时播放提示音", "エージェントがあなたを必要とするとき音を鳴らします")
add("Show detail on lock screen", "在锁屏上显示详情", "ロック画面に詳細を表示")

# Add Connection
add("Multiplexer", "终端复用器", "マルチプレクサ")
add("mosh-server path", "mosh-server 路径", "mosh-server のパス")
# SOCKS5 proxy
add("PROXY", "代理", "プロキシ")
add("Proxy Host", "代理主机", "プロキシのホスト")
add("Proxy Port", "代理端口", "プロキシのポート")
# Under a port field whose text is not a port (B294).
# A labelled field Save needs filled, in its empty state (B319).
add("Required", "必填", "必須")
# A host's emoji (the connection form, §4.13).
add("Emoji", "表情", "絵文字")
add("Shows on the host's mark in place of its key's pattern.", "主机头像上显示这个表情，不再用主机密钥生成的图案。", "ホストのアイコンに、ホスト鍵から作った模様の代わりに表示します。")
add("One emoji, or leave it empty.", "只能填一个表情，也可以留空。", "絵文字を 1 つ入力するか、空のままにしてください。")
add("Just the host — the user and port have their own fields.", "只填主机名或地址，用户和端口各有一栏。", "ホスト名かアドレスだけを入力してください。ユーザーとポートは別の欄です。")

# Agent notifications (actionable, from the Island / lock screen)
add("Send", "发送", "送信")
add("Enable agent notifications on %@?",
    "在 %@ 上启用智能体通知？",
    "%@ でエージェント通知を有効にしますか？")
add("Enable", "启用", "有効にする")
add("Not Now", "以后再说", "今はしない")
add("Don't Ask Again", "不再询问", "今後確認しない")
add("Moshpit installs its hook scripts in ~/.moshpit and registers them in Claude Code's settings, so agents can reach you when they need you. Everything can be removed from Host Setup.",
    "Moshpit 会把状态脚本装到 ~/.moshpit，并登记到 Claude Code 的设置里，智能体在等你时就能通知到你。这些都可以在「主机设置」里移除。",
    "Moshpit は ~/.moshpit にフックスクリプトをインストールし、Claude Code の設定に登録します。エージェントがあなたを必要とするとき届くようにするためです。すべてホスト設定から削除できます。")

# Island hooks installer

# Host key verification (Components)
add("active", "使用中", "使用中")
add("SHA256 fingerprint", "SHA256 指纹", "SHA256 フィンガープリント")
add("Stored", "已保存", "保存済み")
add("Offered now", "本次收到", "今回提示された値")
add("Verify on server", "在服务器上核对", "サーバー側で確認")
add("Host Key Changed", "主机密钥已变更", "ホストキーが変更されました")
# What a connection waiting on the person says while the app is in the background (B632)
add("Confirm %@'s host key to finish connecting.",
    "确认 %@ 的主机密钥后才能连上。",
    "%@ のホストキーを確認すると接続が完了します。")
add("%@'s host key has changed. Open Moshpit to decide.",
    "%@ 的主机密钥变了，打开 Moshpit 决定要不要信任。",
    "%@ のホストキーが変わりました。Moshpit を開いて判断してください。")
add("Enter the passphrase of the key for %@ to finish connecting.",
    "输入连接 %@ 用的密钥口令后才能连上。",
    "%@ に使う鍵のパスフレーズを入力すると接続が完了します。")
add("%@ asks a question before you can sign in. Open Moshpit to answer.",
    "登录 %@ 前要回答一个问题，打开 Moshpit 回答。",
    "%@ へのサインインの前に質問があります。Moshpit を開いて答えてください。")
# The New Host card of a server that offers nothing better than ssh-rsa (B564)
add("This server uses an older host-key algorithm (%@, SHA-1).",
    "这台服务器用的是较旧的主机密钥算法（%@，SHA-1）。",
    "このサーバーは古いホストキーのアルゴリズム（%@、SHA-1）を使っています。")
# A new host this phone connected to before, its trusted key gone with a
# restore or a re-sign (B783)
add("You've connected to this server before, but its key isn't saved on this iPhone. Compare the fingerprint with the server before trusting it.",
    "你以前连过这台服务器，但这台 iPhone 上没存它的密钥。信任之前，先和服务器上的指纹核对一下。",
    "このサーバーには以前接続したことがありますが、この iPhone にはそのキーが保存されていません。信頼する前に、サーバー側のフィンガープリントと照らし合わせてください。")
add("First connection to %@:%@. Verify this fingerprint matches the server before you trust it.",
    "第一次连接 %@:%@。先到服务器上核对指纹，确认一致再信任。",
    "%@:%@ への初回接続です。信頼する前に、このフィンガープリントがサーバーと一致することを確認してください。")
add("The key for %@:%@ does **not** match what's stored here. The server may have been reinstalled — or the connection is being intercepted.",
    "%@:%@ 的密钥和这里保存的**不一样**。可能是服务器重装过，也可能有人在中间截连接。",
    "%@:%@ のキーは、ここに保存されているものと**一致しません**。サーバーが再インストールされた可能性——あるいは接続が傍受されている可能性があります。")

# Multiplexer vocabulary (tmux vs herdr wording)
add("Session", "会话", "セッション")
add("Window", "窗口", "ウィンドウ")
add("Workspace", "工作区", "ワークスペース")
add("Workspaces", "工作区", "ワークスペース")
add("Tab", "标签页", "タブ")
add("Tabs", "标签页", "タブ")
# Mid-sentence forms of the multiplexer nouns ("Kill session “x”?", "No
# sessions yet"). Separate keys rather than `.lowercased()` in code: German
# capitalises nouns, CJK has no case.
add("session", "会话", "セッション")
add("sessions", "会话", "セッション")
add("window", "窗口", "ウィンドウ")
add("windows", "窗口", "ウィンドウ")
add("workspace", "工作区", "ワークスペース")
add("workspaces", "工作区", "ワークスペース")
add("tab", "标签页", "タブ")
add("tabs", "标签页", "タブ")
add("Kill", "终止", "強制終了")
add("Close", "关闭", "閉じる")
add("Loading", "正在加载", "読み込み中")

# ---------- A host's sessions (ui/src/sessions, R7) ----------
add("Attached", "当前", "使用中")
add("current", "当前", "使用中")
add("Disconnected", "已断开", "切断されました")
add("No tmux sessions on this host", "这台主机上没有 tmux 会话", "このホストには tmux セッションがありません")
add("No herdr workspaces on this host", "这台主机上没有 herdr 工作区", "このホストには herdr ワークスペースがありません")
add("New Session", "新建会话", "新規セッション")
add("New Workspace", "新建工作区", "新規ワークスペース")
# "Needs you" and "%lld waiting" are Home's (R6, above): the same words.
add("Working", "工作中", "作業中")
add("Finished", "已完成", "完了")
add("Idle", "空闲", "待機中")
plural("%lld more", "%lld more", "另外 %lld 个", "ほか %lld 件")
add("shell", SAME)
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
add("Pane", "窗格", "ペイン")
# Home tree, window-row long-press: split a new pane off the window.
add("New Pane", "新建窗格", "新しいペイン")
# "✓ %@ finished" lives in the PUSH table below: the notification service
# extension renders the same card and needs the key in its own catalog.

# APNs fallback text. These are not shown by any Swift code: the push relay puts
# them in the payload as `title-loc-key` / `loc-key`, and iOS resolves them
# against this catalog. They surface only when the notification service
# extension fails to decrypt in time — see docs/PUSH.md. The keys ARE the
# English, so a missed lookup still reads as a sentence.
add("pane %lld", "窗格 %lld", "ペイン %lld")

# Multiplexer choice (Add Connection)
add("None", "不使用", "使用しない")
add("Single shell, no session persistence", "只有一个 shell，断开就没了", "シェル 1 つのみ、セッションは保持されません")
add("Mature, already on nearly every host", "成熟，几乎每台主机上都有", "成熟しており、ほぼすべてのホストに導入済み")
add("Built for coding agents — agent status needs no hooks",
    "专为编程智能体设计，不装状态脚本也能看到状态", "コーディングエージェント向け——エージェントの状態にフックは不要")

# Host / install banners
add("herdr isn't installed on this host.", "这台主机上没装 herdr。", "このホストに herdr がインストールされていません。")
add("mosh isn't installed on this host.", "这台主机上没装 mosh。", "このホストに mosh がインストールされていません。")
add("Installs to ~/.local/bin. Moshpit looks there when probing and launching, so you don't need to change PATH.",
    "安装到 ~/.local/bin。Moshpit 探测和启动时都会去那里找，所以你不用改 PATH。",
    "~/.local/bin にインストールされます。Moshpit は検出時も起動時もそこを参照するため、PATH を変更する必要はありません。")

# SSH keys
add("Select Key", "选择密钥", "キーを選択")
add("SAVED KEYS", "已保存的密钥", "保存済みのキー")
add("Enter a private key by hand", "手动输入私钥", "秘密鍵を手入力")
add("Import File…", "导入文件…", "ファイルを読み込む…")
add("OR", "或", "または")
add("Copy Public Key", "复制公钥", "公開鍵をコピー")
add("Share Public Key", "分享公钥", "公開鍵を共有")
add("Copy Fingerprint", "复制指纹", "フィンガープリントをコピー")

# Shortcut editor
add("QUICK KEYS", "快捷键预设", "クイックキー")
add("PREFIX",
    "前缀键",
    "プレフィックス")
add("CONTROL",
    "控制键",
    "コントロール")
add("SPECIAL",
    "特殊键",
    "特殊キー")
add("NAVIGATION",
    "移动",
    "移動")
add("F-KEYS",
    "功能键",
    "ファンクションキー")
add("Interrupt",
    "中断",
    "割り込み")
add("EOF",
    "输入结束",
    "入力終了")
add("Suspend",
    "挂起",
    "一時停止")
add("Quit",
    "退出",
    "終了")
add("Clear",
    "清屏",
    "画面をクリア")
add("Line start",
    "行首",
    "行頭")
add("Line end",
    "行尾",
    "行末")
add("Search",
    "搜历史",
    "履歴を検索")
add("Del word",
    "删前一个词",
    "前の単語を削除")
add("Del line",
    "删到行首",
    "行頭まで削除")
add("Kill EOL",
    "删到行尾",
    "行末まで削除")
add("Yank",
    "贴回刚删的",
    "削除した文字を貼り戻す")
add("Prev",
    "上一条",
    "前へ")
add("Next",
    "下一条",
    "次へ")
add("Tab key",
    "Tab 键",
    "Tab キー")
add("Back-tab",
    "反向 Tab",
    "逆 Tab")
add("Escape",
    "Esc 键",
    "Esc キー")
add("Forward delete",
    "向后删除",
    "右側を削除")
add("Backspace",
    "退格",
    "バックスペース")
add("Left",
    "左",
    "左")
add("Down",
    "下",
    "下")
add("Up",
    "上",
    "上")
add("Right",
    "右",
    "右")
add("Home",
    "Home 键",
    "Home キー")
add("End",
    "End 键",
    "End キー")
add("Page up",
    "上翻一页",
    "前のページ")
add("Page down",
    "下翻一页",
    "次のページ")
add("Interrupt (SIGINT)",
    "中断（SIGINT）",
    "割り込み（SIGINT）")
add("Clear screen",
    "清屏",
    "画面をクリア")
add("Paste clipboard",
    "粘贴剪贴板内容",
    "クリップボードから貼り付け")
add("Tap arrows",
    "点按方向键",
    "タップ式の矢印キー")
add("End of file",
    "输入结束（EOF）",
    "入力終了（EOF）")
add("Reverse search",
    "往回搜历史命令",
    "履歴をさかのぼって検索")
add("Jump to end",
    "跳到最后",
    "末尾へ移動")
add("Back-tab / toggle mode",
    "反向 Tab / 切换模式",
    "逆 Tab / モード切り替え")
add("History prev",
    "上一条历史命令",
    "前の履歴")
add("History next",
    "下一条历史命令",
    "次の履歴")
add("Cursor left",
    "光标左移",
    "カーソルを左へ")
add("Cursor right",
    "光标右移",
    "カーソルを右へ")
add("Clear line before cursor",
    "删掉光标前的内容",
    "カーソルより前を削除")
add("Clear line after cursor",
    "删掉光标后的内容",
    "カーソルより後ろを削除")
add("Beginning of line",
    "移到行首",
    "行頭へ移動")
add("End of line",
    "移到行尾",
    "行末へ移動")
add("Delete word",
    "删前一个词",
    "前の単語を削除")
add("multiplexer prefix", "复用器前缀", "マルチプレクサのプレフィックス")
add("Only in a multiplexer", "只在 tmux / herdr 里显示", "マルチプレクサ内のみ")
add("Show only while the session runs tmux or herdr",
    "仅当会话在跑 tmux 或 herdr 时显示", "セッションが tmux または herdr を実行中のときのみ表示します")
add("Tap a preset to fill the trigger — tmux prefixes, control chords, special & navigation keys, F-keys. Then tweak the chip label / color below.",
    "点一个预设就填好触发键：tmux 前缀、Control 组合、特殊键和方向键、F 键。然后在下面改标签文字和颜色。",
    "プリセットをタップするとトリガーが入力されます——tmux プレフィックス、Control コード、特殊キーと移動キー、F キー。その後、下でチップのラベルと色を調整します。")
add("In Key Combo mode the payload is the escape sequence sent to the PTY, honoring the transport transcription rules.",
    "「按键组合」模式下，内容就是发给 PTY 的转义序列，会按传输方式的转写规则处理。",
    "「キーの組み合わせ」モードでは、ペイロードは PTY に送るエスケープシーケンスであり、トランスポートの変換ルールに従います。")

# Themes / icons
add("Untitled", "未命名", "無題")
add("No themes found in that JSON.", "那段 JSON 里没有找到主题。", "その JSON にテーマが見つかりませんでした。")
add("That doesn't look like theme JSON. Paste an exported theme, or an object with \"background\", \"foreground\" and the eight ANSI color names.",
    "这看起来不是主题 JSON。请粘贴导出的主题，或一个带 “background”、“foreground” 和八个 ANSI 颜色名的对象。",
    "テーマの JSON ではないようです。書き出したテーマ、または \"background\"・\"foreground\" と 8 つの ANSI カラー名を持つオブジェクトを貼り付けてください。")
add("Couldn't change the icon: %@", "无法更换图标：%@", "アイコンを変更できませんでした：%@")
add("The icon is separate from the accent color, because iOS only allows switching between icons bundled with the app — a custom accent can't have matching artwork generated for it.",
    "图标和强调色分开选，是因为 iOS 只允许在 App 自带的几个图标之间切换，没法给自定义的强调色现做一个配套图标。",
    "アイコンとアクセントカラーは別の設定です。iOS はアプリに同梱されたアイコン間の切り替えしか許可しないため、カスタムアクセントに合わせた画像を生成することはできません。")

# Connection / session failures
add("Not attached to the pane yet — input wasn't sent",
    "还没连上这个窗格，刚才的输入没发出去", "まだペインに接続していません。入力は送信されませんでした")
add("That pane is closed", "那个窗格已经关了", "そのペインはもう閉じています")
add("Couldn't switch to that pane", "切不到那个窗格", "そのペインに切り替えられませんでした")
add("Couldn't switch to that pane — nothing was sent",
    "没能切到那个窗格，什么都没发出去", "そのペインに切り替えられませんでした。何も送信していません")
# A plain shell's keys typed while the way back found the connection dead (B396)
add("What you just typed wasn't sent: the connection dropped, and it's back now",
    "刚才输入的内容没有发出去：连接已断开，现在已重新连上",
    "さっき入力した内容は送信されませんでした。接続が切れていましたが、つなぎ直しました")
add("This pane is open on another device",
    "这个窗格已在另一台设备上打开", "このペインは別のデバイスで開かれています")
add("Open here", "在本机打开", "このデバイスで開く")
add("Couldn't reach the host", "无法连到主机", "ホストに到達できませんでした")
add("Creating the worktree failed", "创建工作树失败", "ワークツリーの作成に失敗しました")
add("herdr didn't confirm the worktree, so no agent was started. Look for a “%@” workspace before trying again.",
    "herdr 没说工作树建没建成，智能体也就没启动。重试之前，先看看有没有叫“%@”的工作区。",
    "herdr からワークツリー作成の確認が返ってこなかったため、エージェントは起動していません。やり直す前に「%@」というワークスペースができていないか確認してください。")
add("The host stopped answering while the worktree was being made, so no agent was started. Look for a “%@” workspace before trying again.",
    "建工作树的时候主机没了回应，智能体也就没启动。重试之前，先看看有没有叫“%@”的工作区。",
    "ワークツリーの作成中にホストが応答しなくなったため、エージェントは起動していません。やり直す前に「%@」というワークスペースができていないか確認してください。")
add("The worktree is ready, but the host stopped answering before %@ started.",
    "工作树建好了，但 %@ 还没启动，主机就没了回应。",
    "ワークツリーは用意できましたが、%@ が起動する前にホストが応答しなくなりました。")
add("Cancelled. The worktree is ready, but no agent was started in it.",
    "已取消。工作树已经建好，但里面没有启动智能体。",
    "キャンセルしました。ワークツリーは作成済みですが、エージェントは起動していません。")
add("Removing the worktree failed", "删除工作树失败", "ワークツリーの削除に失敗しました")
add("The host stopped answering while the worktree was being removed.",
    "删除工作树的时候主机没了回应。",
    "ワークツリーの削除中にホストが応答しなくなりました。")
add("The worktree was created but has no pane", "工作树已创建，但没有窗格", "ワークツリーは作成されましたが、ペインがありません")
add("The worktree is ready, but %@ didn't start: %@",
    "工作树建好了，但 %@ 没能启动：%@",
    "ワークツリーは用意できましたが、%@ が起動しませんでした：%@")
add("The task is running, but %@ didn't get the first message: %@",
    "任务已经开始，但 %@ 没收到首条消息：%@",
    "タスクは開始しましたが、%@ に最初のメッセージが届きませんでした：%@")
add("The task is running, but %@ wasn't ready for the first message in time.",
    "任务已经开始，但 %@ 迟迟没就绪，首条消息没发出去。",
    "タスクは開始しましたが、%@ の準備が間に合わず、最初のメッセージを送れませんでした。")
add("The task is running, but the host stopped answering before the first message was sent.",
    "任务已经开始，但首条消息还没发出去，主机就没了回应。",
    "タスクは開始しましたが、最初のメッセージを送る前にホストが応答しなくなりました。")
add("herdr exited with status %lld", "herdr 退出了，状态码 %lld", "herdr が終了しました（ステータス %lld）")
add("herdr's answer was cut off", "herdr 的回应半路断了", "herdr の応答が途中で切れました")
add("The task is running, but the first message is still in %@'s input: pressing Return failed (%@). Press Return in the pane to send it.",
    "任务已经开始，但首条消息还停在 %@ 的输入框里：回车没按下去（%@）。到窗格里按一下回车就能发出去。",
    "タスクは開始しましたが、最初のメッセージは %@ の入力欄に残ったままです。Return を押せませんでした（%@）。ペインで Return を押すと送信されます。")
# A saved password or key this device doesn't hold: a phone restored from a backup (B782)
add("The password for %@ isn't on this iPhone. Enter it again.",
    "这台 iPhone 上没有 %@ 的密码，请重新输入。",
    "この iPhone には %@ のパスワードがありません。もう一度入力してください。")
add("The private key for %@ isn't on this iPhone. Paste it again.",
    "这台 iPhone 上没有 %@ 的私钥，请重新粘贴。",
    "この iPhone には %@ の秘密鍵がありません。もう一度ペーストしてください。")
add("The key %@ signs in with isn't on this iPhone. Open Edit to pick another key.",
    "这台 iPhone 上没有 %@ 登录用的密钥。打开「编辑」换一把密钥。",
    "%@ のサインインに使うキーがこの iPhone にありません。「編集」を開いて別のキーを選んでください。")
add("The saved password or key isn't on this iPhone. Open Edit and enter it again.",
    "这台 iPhone 上没有保存的密码或密钥。打开「编辑」重新输入。",
    "保存したパスワードまたはキーがこの iPhone にありません。「編集」を開いてもう一度入力してください。")
add("While this device is locked, Moshpit can't read the server keys you've trusted, so it can't check this server. Unlock it and connect again.",
    "设备锁着的时候，Moshpit 读不到你信任过的服务器密钥，也就没法核对这台服务器。解锁之后再连一次。",
    "デバイスがロックされている間は、信頼済みのサーバーキーを読み出せないため、このサーバーを確認できません。ロックを解除してから接続し直してください。")
add("You didn't trust the server's key, so Moshpit didn't connect.",
    "你没有信任这台服务器的主机密钥，所以 Moshpit 没有连接。",
    "サーバーのホストキーを信頼しなかったため、接続しませんでした。")
add("The connection failed for a reason Moshpit didn't recognise. Check the host, port and that the server is reachable, then try again.",
    "连接失败，Moshpit 也没认出是什么原因。检查一下主机、端口，确认服务器能连通，再试一次。",
    "Moshpit が識別できない理由で接続に失敗しました。ホスト・ポート・サーバーに到達できるかを確認して、もう一度お試しください。")
add("The server closed the connection while setting up SSH. Check that the port really is an SSH server, and that a firewall or proxy isn't cutting the connection.",
    "SSH 还没建好，服务器就把连接关了。确认这个端口跑的确实是 SSH，中间也没有防火墙或代理把连接掐断。",
    "SSH の確立中にサーバーが接続を閉じました。そのポートが本当に SSH サーバーであること、ファイアウォールやプロキシが接続を切断していないことを確認してください。")

# ---------- Strings the compiler extracts that had no entry yet (2026-09-14) ----------
# Found by diffing .stringsdata against the catalog; without these the zh / ja
# UI silently showed English. Format-only keys are SAME.
# Attach-image intent
add("Agents", "智能体", "エージェント")
add("Images", "图片", "画像")
add("No readable images were provided.", "没有收到可读取的图片。", "読み取れる画像がありませんでした。")
add("Queued for %@ — delivers when the session is next live.", "已排队，等 %@ 的会话下次在线时送达。", "%@ にキュー済み。セッションが次にオンラインになったら届けます。")
# A queued share held back: its pane no longer runs the agent (B622).
add("A share wasn't delivered", "有 1 个分享没送出去", "共有を 1 件届けられませんでした")
add("That pane isn't running %@ any more.", "那个窗格里已经不是 %@ 了。", "そのペインではもう %@ が動いていません。")
add("That pane is closed.", "那个窗格已经关了。", "そのペインはもう閉じています。")
# Home's notice for a held share (B622): what happened, Send to…, Discard.
add("A share wasn't delivered: that pane isn't running %@ any more.",
    "有 1 个分享没送出去：那个窗格里已经不是 %@ 了。",
    "共有を 1 件届けられませんでした。そのペインではもう %@ が動いていません。")
add("A share wasn't delivered: that pane is closed.",
    "有 1 个分享没送出去：那个窗格已经关了。",
    "共有を 1 件届けられませんでした。そのペインはもう閉じています。")
add("Send to…", "换个窗格…", "送り先を選ぶ…")
add("Discard", "丢掉", "破棄")
add("Panes running %@ come first.", "跑着 %@ 的窗格排在前面。", "%@ が動いているペインを先に並べています。")
add("Send Image to Agent", "把图片发给智能体", "エージェントに画像を送信")
add("Sent to %@.", "已发给 %@。", "%@ に送信しました。")
add("Uploads images to the agent's server and hands it the file paths.", "把图片上传到智能体所在的服务器，再把文件路径交给它。", "画像をエージェントのサーバーにアップロードし、ファイルパスを渡します。")
# Hook config / installer
add("Codex won't run a newly added hook until you trust it. Run /hooks inside Codex on this host and trust the four Moshpit entries — until you do, nothing will be sent.",
    "Codex 不会运行你还没信任的新脚本。在这台主机上打开 Codex，运行 /hooks，把 Moshpit 的四条状态脚本设为信任。在那之前，一条通知也发不出来。",
    "Codex は信頼されるまで新しく追加されたフックを実行しません。このホストの Codex で /hooks を実行し、Moshpit の 4 つのエントリを信頼してください。それまでは何も送信されません。")
add("That config file isn't valid JSON (%@). Fix it on the host, or move it aside and Moshpit will write a fresh one.",
    "这个配置文件不是合法的 JSON（%@）。到主机上修好它，或者先挪走，Moshpit 会重新写一份。",
    "この設定ファイルは有効な JSON ではありません（%@）。ホスト上で修正するか、いったん退避させれば Moshpit が新しく書き出します。")
add("That config file's top level isn't a JSON object, so Moshpit can't merge hooks into it.",
    "这个配置文件的顶层不是 JSON 对象，Moshpit 没法把状态脚本写进去。",
    "この設定ファイルのトップレベルが JSON オブジェクトではないため、Moshpit はフックをマージできません。")
add("The manifest on this host names a config path Moshpit won't run a command against: %@",
    "这台主机上的清单指向了一个 Moshpit 不会对它执行命令的配置路径：%@",
    "このホストのマニフェストには、Moshpit がコマンドを実行しない設定パスが指定されています：%@")
# Push service
add("Couldn't save the pairing on this phone, so nothing was installed on the host. The diagnostics log has the reason.",
    "配对信息没能保存到这台手机上，所以主机那边什么都没装。原因在诊断日志里。",
    "この iPhone にペアリングを保存できなかったため、ホストには何もインストールされていません。理由は診断ログにあります。")
add("Couldn't remove the pairing from this phone, so the host was left paired. The diagnostics log has the reason.",
    "没能删掉这台手机上的配对信息，所以主机那边也没动，仍然保持配对。原因在诊断日志里。",
    "この iPhone のペアリング情報を削除できなかったため、ホスト側も変更せず、ペアリングされたままです。理由は診断ログにあります。")
add("Relay refused to mint a send token (%lld): %@", "推送服务拒绝签发发送令牌（%lld）：%@", "リレーが送信トークンの発行を拒否しました（%lld）：%@")
add("That relay address isn't a valid URL.", "推送服务的地址不是有效的 URL。", "このリレーのアドレスは有効な URL ではありません。")
add("iOS hasn't issued a push token to this phone yet — check that notifications are allowed, then try again.",
    "iOS 还没给这台手机发推送令牌。确认已允许通知，再试一次。",
    "iOS がまだこの iPhone にプッシュトークンを発行していません。通知が許可されているか確認して、もう一度お試しください。")
# herdr server
add("Restart didn't take — on the host, run: herdr server stop && herdr", "重启没生效。到主机上运行：herdr server stop && herdr", "再起動が反映されませんでした。ホストで次を実行してください：herdr server stop && herdr")
add("Restart herdr server", "重启 herdr 服务", "herdr サーバーを再起動")
add("Restarting herdr server…", "正在重启 herdr 服务…", "herdr サーバーを再起動中…")
add("Retry", "重试", "再試行")
add("Retrying in %lld s", "%lld 秒后重试", "%lld 秒後に再試行")
add("Restart server", "重启服务", "サーバーを再起動")
add("herdr on the host's PATH is %@, its server %@ — update that copy", "主机 PATH 里的 herdr 是 %1$@，服务是 %2$@，升级 PATH 里那份就好", "ホストの PATH 上の herdr は %1$@、サーバーは %2$@ です。PATH 上の herdr を更新してください")
add("The host's herdr server is %@, older than its herdr %@", "主机上运行的 herdr 服务是 %1$@，比 herdr 本身（%2$@）旧", "ホストの herdr サーバーは %1$@ で、herdr 本体（%2$@）より古いです")
# Home
add("New", "新建", "新規")
add("Connections", "连接", "接続")
add("More", "更多", "その他")
add("No Terminal Open", "没有打开的终端", "ターミナルが開いていません")
add("Pick a host from the sidebar, or add one with ⌘N.", "从边栏选一台主机，或按 ⌘N 添加。", "サイドバーからホストを選ぶか、⌘N で追加してください。")
add("The pit never closes", "机器不眠", "ピットは眠らない")
# Host setup
add("Enter the address of your push relay first.", "先填推送服务的地址。", "まずプッシュリレーのアドレスを入力してください。")
add("Installing on %@…", "正在安装到 %@…", "%@ にインストール中…")
add("Minting a send token at the relay…", "正在向推送服务申请发送令牌…", "リレーで送信トークンを発行中…")
add("Open a tmux pane on this host to test the hooks.", "先在这台主机上打开一个 tmux 窗格，才能测试状态脚本。", "フックをテストするには、このホストで tmux のペインを開いてください。")
add("The host reported: %@", "主机返回：%@", "ホストからの報告：%@")
add("The host sent one, but this screen never saw it arrive. Most likely Moshpit was not in the foreground — the test can only be confirmed while you are looking at this screen, so try again without leaving it. If it still fails, check that notifications are allowed and that the relay can reach Apple.",
    "主机发了一条，但这个页面没收到。多半是 Moshpit 当时不在前台：只有你停在这个页面时，测试才能确认收到，别切走，再试一次。还不行的话，看看通知有没有允许，推送服务能不能连上 Apple。",
    "ホストは 1 件送信しましたが、この画面には届きませんでした。おそらく Moshpit がフォアグラウンドにありませんでした。テストはこの画面を見ている間だけ確認できるので、離れずにもう一度お試しください。それでも失敗する場合は、通知が許可されているか、リレーが Apple に到達できるかを確認してください。")
add("The pane came back as \"%@\" instead of the test stamp.", "窗格返回的是 \"%@\"，不是测试标记。", "ペインからテストスタンプではなく \"%@\" が返ってきました。")
add("The stamp script ran but the pane came back empty.%@", "标记脚本跑了，但窗格返回为空。%@", "スタンプスクリプトは実行されましたが、ペインは空で返ってきました。%@")
add("This host is missing %@ — install those and try again.", "这台主机缺少 %@。装好再试。", "このホストには %@ がありません。インストールしてからもう一度お試しください。")
add("This host's tmux isn't on the default socket, so Moshpit can't run the test. The hooks themselves are unaffected.",
    "这台主机的 tmux 没用默认 socket，Moshpit 跑不了这个测试。状态脚本本身不受影响。",
    "このホストの tmux はデフォルトのソケットを使っていないため、Moshpit はテストを実行できません。フック自体には影響ありません。")
add("Install", "安装", "インストール")
add("Looking at the host…", "正在查看主机…", "ホストを確認中…")
add("Couldn't read this host", "读不了这台主机", "このホストを読み取れませんでした")
add("Not installed", "未安装", "未インストール")
add("Agent status", "智能体状态", "エージェントの状態")
add("Push", "推送", "プッシュ")
add("Installed", "已安装", "インストール済み")
add("Paired", "已配对", "ペアリング済み")
add("Not paired", "未配对", "未ペアリング")
add("Status scripts", "状态脚本", "ステータススクリプト")
add("Operating system", "操作系统", "OS")
add("Passed", "通过", "成功")
add("Failed", "失败", "失敗")
add("Paired, but nothing will push. Install the status scripts below.", "已经配对，但不会有任何推送。先在下面装好状态脚本。", "ペアリングは済んでいますが、プッシュは届きません。下でステータススクリプトをインストールしてください。")
add("Not found on this host", "这台主机上没有", "このホストにはありません")
add("%@ isn't on this host", "这台主机上没有 %@", "このホストに %@ はありません")
add("Moshpit found neither %@ nor a %@ command here. Its hooks will run once it's installed.", "Moshpit 在这里既没找到 %@，也没找到 %@ 命令。等它装好，状态脚本就会运行。", "Moshpit はここで %@ も %@ コマンドも見つけられませんでした。インストールすればフックが動きます。")
add("Install Anyway", "仍然安装", "このままインストール")
add("Out of date", "需要更新", "古くなっています")
add("Pair", "配对", "ペアリング")
add("Pairing", "配对", "ペアリング")
add("Proven — it arrived on this phone.", "验证通过，这台手机收到了。", "確認済み。この iPhone に届きました。")
add("Pushes need %@ on this host.", "推送需要这台主机上有 %@。", "プッシュにはこのホストに %@ が必要です。")
add("Re-pair", "重新配对", "再ペアリング")
add("Reaches you when Moshpit isn't running. Your relay signs the push; it never sees what the agent said.",
    "Moshpit 没在运行也能通知到你。你的推送服务只给通知签名，看不到智能体说了什么。",
    "Moshpit が起動していないときも届きます。リレーはプッシュに署名するだけで、エージェントの内容は見えません。")
add("Reinstall", "重新安装", "再インストール")
add("Send a test notification", "发一条测试通知", "テスト通知を送信")
add("Test the status script", "测试状态脚本", "ステータススクリプトをテスト")
add("Runs the command %@'s config registers for its hooks, in this tmux pane, and reads the pane back. The agent itself isn't started.", "在这个 tmux 窗格里运行 %@ 配置里登记的状态脚本命令，再读回窗格。不会启动智能体本身。", "%@ の設定に登録されたフックのコマンドをこの tmux ペインで実行し、ペインを読み戻します。エージェント自体は起動しません。")
add("Running it in this pane…", "正在这个窗格里运行…", "このペインで実行中…")
add("Works — the pane shows the status it set.", "可以用：窗格显示了它设的状态。", "動作しています。ペインに設定した状態が表示されました。")
add("Install the hooks for %@ to test them.", "先为 %@ 安装状态脚本，才能测试。", "テストするには、先に %@ のフックをインストールしてください。")
add("Update the hooks for %@ first: the test runs what this version of Moshpit installs.", "先更新 %@ 的状态脚本：测试运行的是这个版本的 Moshpit 装的内容。", "先に %@ のフックを更新してください。テストはこのバージョンの Moshpit がインストールするものを実行します。")
add("%@ doesn't register Moshpit for %@. Reinstall to put it back.", "%@ 里没有为 %@ 登记 Moshpit。重新安装就能补上。", "%@ に %@ 用の Moshpit が登録されていません。再インストールすると元に戻ります。")
add("The host sends one for real. If it arrives here, the whole chain works.", "主机会真的发一条。这里收到了，整条链路就是通的。", "ホストが実際に 1 件送信します。ここに届けば、一連の仕組みは正常です。")
add("Try again", "再试一次", "もう一度")
add("This page couldn't open.", "这个页面打不开。", "このページを開けません。")
add("Unpair", "解除配对", "ペアリング解除")
add("Remove Moshpit from this host", "从这台主机移除 Moshpit", "このホストから Moshpit を削除")
add("Remove Moshpit from this host?", "要从这台主机移除 Moshpit 吗？", "このホストから Moshpit を削除しますか？")
add("Moshpit's hooks come out of every agent's config, and this phone's pairing is removed. If no other phone is paired with this host, ~/.moshpit goes too. Everything else in the agents' configs stays.", "每个智能体配置里的 Moshpit 状态脚本都会拿掉，这台手机的配对也会删掉。要是没有别的手机和这台主机配对，~/.moshpit 也会一起删除。智能体配置里的其他内容不动。", "各エージェントの設定から Moshpit のフックを外し、この iPhone のペアリングも削除します。このホストとペアリングしているほかの端末がなければ、~/.moshpit も削除します。エージェントの設定のほかの内容はそのまま残ります。")
add("Update", "更新", "更新")
add("Waiting for it to arrive…", "等它送到…", "到着を待っています…")
# Settings
add("24 hours", "24 小时", "24 時間")
add("7 days", "7 天", "7 日")
add("ATTACHED IMAGES", "附带的图片", "添付画像")
# Shortcuts
add("Add %@ to toolbar", "把 %@ 加到快捷键栏", "%@ をツールバーに追加")
# The shortcut editor's edit controls on the NSH page (W11e): each is
# labelled for screen readers, and the − of a shortcut in the bar says what it
# does there (it takes it off the bar; the SwiftUI page's Delete said it was
# deleted).
add("Reorder %@", "调整 %@ 的顺序", "%@ を並べ替え")
add("Delete %@", "删除 %@", "%@ を削除")
add("Remove %@ from toolbar", "把 %@ 从快捷键栏拿掉", "%@ をツールバーから外す")
add("Tap ＋ in the top right to create a custom shortcut — key combos, text snippets, and command chains. Tap one to edit it.",
    "点右上角的 ＋ 新建快捷键，可以是按键组合、一段文本，或一串命令。点已有的可以编辑。",
    "右上の ＋ でカスタムショートカットを作成できます。キーの組み合わせ、テキスト、コマンドの連鎖。タップすると編集できます。")
# Hardware keys / terminal chrome
add("Back to Home", "回到主页", "ホームに戻る")
add("Terminal", "终端", "ターミナル")
add("Window %@", "窗口 %@", "ウインドウ %@")
add("PREPARING…", "准备中…", "準備中…")
add("READY TO INSERT", "可以插入", "挿入できます")
add("Reading the selected images…", "正在读取选中的图片…", "選択した画像を読み込み中…")
add("UPLOAD FAILED", "上传失败", "アップロード失敗")
add("UPLOADING %lld%%", "上传中 %lld%%", "アップロード中 %lld%%")

# ---------- Host path preview (RemotePreview) ----------
add("OPENING…", "打开中…", "開いています…")
add("DOWNLOADING %lld%%", "下载中 %lld%%", "ダウンロード中 %lld%%")
add("LARGE FILE", "文件较大", "大きなファイル")
add("CAN'T OPEN", "打不开", "開けません")
add("This file is %@. Download it to preview?", "这个文件有 %@，要下载下来预览吗？",
    "このファイルは %@ です。ダウンロードしてプレビューしますか？")
add("This file is no longer on the host.", "这个文件已经不在主机上了。", "このファイルはもうホストにありません。")
add("This file changed while it was being fetched. Try again.",
    "这个文件在下载时被改动了，再试一次。",
    "このファイルはダウンロード中に変更されました。もう一度お試しください。")
add("This host has SFTP turned off, and the file is too large to send without it.",
    "这台主机关了 SFTP，文件又太大，没法用别的办法传过来。",
    "このホストは SFTP がオフで、SFTP なしで送るにはファイルが大きすぎます。")
add("Couldn't reach the host. Check the connection and try again.",
    "连不上主机。检查一下连接再试。", "ホストに接続できません。接続を確認してもう一度お試しください。")
add("Only images, video, audio, PDFs and documents are shown here.",
    "这里只显示图片、视频、音频、PDF 和文档。",
    "ここには画像、ビデオ、オーディオ、PDF、書類だけが表示されます。")
# Folder sheet: add / rename / delete
add("Add", "添加", "追加")
add("New Folder", "新建文件夹", "新規フォルダ")
add("Couldn't upload", "没能上传", "アップロードできませんでした")
add("Couldn't create the folder", "没能新建文件夹", "フォルダを作成できませんでした")
add("Couldn't rename", "没能重命名", "名前を変更できませんでした")
add("Couldn't delete", "没能删除", "削除できませんでした")
add("Couldn't read that photo.", "读不了这张照片。", "この写真を読み込めませんでした。")
add("Something with that name is already there.", "这里已经有同名的文件或文件夹了。", "同じ名前のものがすでにあります。")
add("The host didn't allow it. Check that you can write to this folder.", "主机拒绝了这次操作。检查一下你对这个文件夹有没有写权限。", "ホストに拒否されました。このフォルダに書き込めるか確認してください。")
add("“%@” and everything in it will be deleted from the host.", "会从主机上删除 %@ 和里面的所有内容。", "ホストから %@ とその中身がすべて削除されます。")
add("“%@” will be deleted from the host.", "会从主机上删除 %@。", "ホストから %@ が削除されます。")
# Shortcut bar: the Files key
add("Browse files", "浏览文件", "ファイルを見る")
add("Share", "分享", "共有")
add("Attach image", "附带图片", "画像を添付")
add("Back", "返回", "戻る")
add("Session ended", "会话已结束", "セッション終了")
add("Paste image from clipboard", "从剪贴板粘贴图片", "クリップボードから画像を貼り付け")
add("Repaint screen", "重绘屏幕", "画面を再描画")
add("Sent this session", "本次会话已发送", "このセッションで送信済み")
add("Take photo", "拍照", "写真を撮る")
add("Toggle Sidebar", "显示/隐藏边栏", "サイドバーを切り替え")
add("input held until the server answers", "输入暂缓发送，等服务器应答", "サーバーの応答まで入力を保留中")

# ---------- Formerly Xcode-kept keys, now curated (2026-09-14) ----------
# Glyphs, paths, numbers and code fragments are SAME; the rest carry rewritten zh.
add("Arrow keys", "方向键", "矢印キー")
add("Authentication failed — the server rejected your username and password / key. Double-check your credentials.", "登录失败，服务器不接受这个用户名和密码或密钥。检查一下填的对不对。", "認証に失敗しました — ユーザー名とパスワード / 鍵がサーバーに拒否されました。認証情報を確認してください。")
add("Base")
add("Can't re-check over this transport — reconnect to apply.", "当前传输方式下没法重新检测。重新连接后生效。", "このトランスポートでは再チェックできません — 再接続して適用してください。")
add("Connected, but the server wouldn't open a session channel.", "连上了，但服务器不肯打开会话通道。", "接続しましたが、サーバーがセッションチャネルを開きませんでした。")
add("Connection has no associated keychain reference", "这个连接没有关联的钥匙串条目", "接続に関連付けられたキーチェーン参照がありません")
add("Copied", "已复制", "コピーしました")
add("Copy command", "复制命令", "コマンドをコピー")
add("Couldn't detect a package manager.", "没找到包管理器。", "パッケージマネージャを検出できませんでした。")
add("Couldn't reach the server. Check the host, port, and that it's online.", "连不上服务器。检查一下主机、端口，以及服务器是否在线。", "サーバーに接続できませんでした。ホスト、ポート、サーバーが稼働しているか確認してください。")
# B740: an unreachable host, worded by why; a local one names the Local Network switch.
add("%@ refused the connection on port %@. Check the port, and that SSH is running there.",
    "%1$@ 拒绝了端口 %2$@ 上的连接。检查一下端口，以及那台机器上的 SSH 有没有在运行。",
    "%1$@ がポート %2$@ への接続を拒否しました。ポート番号と、SSH が動いているか確認してください。")
add("Couldn't find %@ on your local network. Check the name, and that Moshpit may use the local network: Settings → Privacy & Security → Local Network.",
    "在本地网络里找不到 %@。检查一下名字，并确认 Moshpit 可以使用本地网络：「设置 → 隐私与安全性 → 本地网络」。",
    "ローカルネットワーク上に %@ が見つかりません。名前と、Moshpit にローカルネットワークの使用を許可しているかを確認してください（「設定 → プライバシーとセキュリティ → ローカルネットワーク」）。")
add("Couldn't find %@. Check the host name.",
    "找不到 %@。检查一下主机名。",
    "%@ が見つかりません。ホスト名を確認してください。")
add("Couldn't reach %@ on your local network. Check that it's on, and that Moshpit may use the local network: Settings → Privacy & Security → Local Network.",
    "连不上本地网络里的 %@。确认它开着机，并确认 Moshpit 可以使用本地网络：「设置 → 隐私与安全性 → 本地网络」。",
    "ローカルネットワーク上の %@ に接続できません。電源が入っているかと、Moshpit にローカルネットワークの使用を許可しているかを確認してください（「設定 → プライバシーとセキュリティ → ローカルネットワーク」）。")
add("%@ didn't answer. Check the address, that the host is on, and your VPN if it sits behind one.",
    "%@ 没有响应。检查一下地址、主机有没有开机；如果它在 VPN 后面，再看看 VPN 连上没有。",
    "%@ から応答がありません。アドレスとホストの電源を確認してください。VPN の内側にあるなら VPN の接続も確認してください。")
add("There's no route to %@ from this network. Check your connection, and your VPN if the host sits behind one.",
    "当前网络没有通往 %@ 的路由。检查一下网络连接；如果主机在 VPN 后面，再看看 VPN 连上没有。",
    "このネットワークから %@ への経路がありません。接続状況を確認してください。VPN の内側にあるホストなら VPN の接続も確認してください。")
# B801: a failure on a jump host's leg (F1, the jump-host feature) says which first.
add("Jump host “%@”: %@", "跳板主机「%1$@」：%2$@", "踏み台「%1$@」：%2$@")
add("Couldn't reach the SOCKS proxy at %@. Check that it's running, and its host and port.",
    "连不上 %@ 上的 SOCKS 代理。看看代理有没有在运行，主机和端口有没有填错。",
    "%@ の SOCKS プロキシに接続できませんでした。プロキシが動いているか、ホストとポートが正しいか確認してください。")
add("The SOCKS proxy didn't connect you to the server. Check that it's a SOCKS5 proxy that needs no password, and that it can reach the server.",
    "SOCKS 代理没能帮你连上服务器。确认它是不需要密码的 SOCKS5 代理，并且能连到这台服务器。",
    "SOCKS プロキシ経由でサーバーに接続できませんでした。パスワード不要の SOCKS5 プロキシか、プロキシからサーバーに届くかを確認してください。")
add("Create", "创建", "作成")
add("Drag toward a direction to send an arrow key", "朝一个方向拖，就发对应的方向键", "方向へドラッグして矢印キーを送信")
add("Feedback", "反馈", "フィードバック")
add("Hide keyboard", "收起键盘", "キーボードを隠す")
add("Install mosh with your platform's package manager (or build from source), then re-check.", "用你平台的包管理器装上 mosh（或者从源码编译），再重新检测。", "お使いのプラットフォームのパッケージマネージャで mosh をインストール（またはソースからビルド）してから、再チェックしてください。")
add("Install on host", "在主机上安装", "ホストにインストール")
add("Install tmux and mosh with your platform's package manager (or build from source), then re-check.", "用你平台的包管理器装上 tmux 和 mosh（或者从源码编译），再重新检测。", "お使いのプラットフォームのパッケージマネージャで tmux と mosh をインストール（またはソースからビルド）してから、再チェックしてください。")
add("Installed — reconnect to enable it.", "已安装。重新连接后启用。", "インストール済み — 再接続して有効にします。")
add("Large v3 Turbo")
add("MOSH")
add("Moshpit")
add("Moshpit never installs anything silently. Run the command below in your shell — sudo and its output stay fully visible.", "Moshpit 不会悄悄装任何东西。把下面的命令贴到你的 shell 里运行，sudo 和输出全程都看得见。", "Moshpit が黙ってインストールすることはありません。以下のコマンドをシェルで実行してください — sudo とその出力はすべて表示されます。")
add("Move down", "下", "下へ")
add("Move left", "左", "左へ")
add("Move right", "右", "右へ")
add("Move up", "上", "上へ")
add("Name (optional)", "名称（可选）", "名前（任意）")
# Recent Log, when the system's log store can't be read (DiagnosticsLog.readFailure).
add("Couldn't read the log: %@", "没能读出日志：%@", "ログを読み込めませんでした：%@")
add("Open terminal", "打开终端", "ターミナルを開く")
add("Re-check", "重新检测", "再チェック")
add("Re-checking…", "正在重新检测…", "再チェック中…")
add("Recent log", "最近日志", "最近のログ")
# R15/R16: the voice pages, Send feedback and Recent log on the redesign.
plural("Copied %lld lines", "Copied %lld line", "已复制 %lld 行", "%lld 行をコピーしました")
add("Couldn't read the log", "读不到日志", "ログを読み込めませんでした")
add("Nothing logged in the last 30 minutes", "最近 30 分钟没有日志", "直近30分のログはありません")
add("Showing the latest 500 of %lld. Copy takes them all.", "只显示最近 500 条，共 %lld 条；复制会带上全部。", "最新の500件を表示しています（全%lld件）。コピーではすべてを含めます。")
add("Message", "内容", "内容")
add("What broke, or what you want", "什么坏了，或者想要什么", "何が壊れたか、何がほしいか")
add("Include device and version", "附上设备和版本", "端末とバージョンを添える")
add("Include recent log", "附上最近日志", "最近のログを添える")
add("The log is the last 30 minutes, the same lines as Recent log.", "日志是最近 30 分钟的，和「最近日志」里的一样。", "ログは直近30分のもので、「最近のログ」と同じ内容です。")
add("Mail isn't set up on this device", "这台设备没设邮件", "この端末ではメールが設定されていません")
add("Copy address", "复制地址", "アドレスをコピー")
add("Open an issue on GitHub", "在 GitHub 上提 issue", "GitHub で Issue を作成")
add("Issues are public; keep security reports to mail.", "issue 是公开的；安全问题请发邮件。", "Issue は公開されます。セキュリティの報告はメールでお願いします。")
add("Built in and quick; one language at a time", "系统自带，快；一次听一种语言", "内蔵で高速。一度に1つの言語のみ")
add("A model on this device; follows speech that mixes languages", "本地模型，中英混说也能听", "端末内のモデル。言語が混ざった話し方にも対応")
add("Both run on this device. Your voice is never uploaded.", "两种都在这台设备上运行，声音不会上传。", "どちらもこの端末で動作し、音声がアップロードされることはありません。")
add("Download a model first", "先下载一个模型", "先にモデルをダウンロードしてください")
add("Search languages", "搜索语言", "言語を検索")
add("Only languages this device can transcribe. Each downloads once, then works offline.", "只列出这台设备能听写的语言。每种第一次用时下载，之后离线也能用。", "この端末で書き起こせる言語だけを表示します。初回に一度ダウンロードすれば、以降はオフラインで使えます。")
add("Picking your language keeps the foreign words you say mid-sentence intact.", "选定语言后，句子里夹的外语词也能原样保留。", "言語を指定すると、文の途中に入る外国語もそのまま残ります。")
add("Common", "常用", "よく使う言語")
add("All languages", "全部语言", "すべての言語")
add("No language matches", "没有匹配的语言", "一致する言語はありません")
add("Downloads once from Hugging Face, then runs on this device. Bigger models handle mixed speech better, but slower.", "从 Hugging Face 下载一次，之后都在这台设备上运行。模型越大，中英混说听得越准，但越慢。", "Hugging Face から一度ダウンロードすれば、あとはこの端末で動作します。大きいモデルほど言語が混ざった話し方に強いですが、遅くなります。")
add("%@ of ≈%@", "已下载 %@，共约 %@", "%@ / 約 %@")
add("In use", "使用中", "使用中")
add("Discard download", "丢弃没下完的部分", "ダウンロードを破棄")
add("More for %@", "%@ 的更多操作", "%@ のその他の操作")
add("Reconnect", "重新连接", "再接続")
add("Reconnecting…", "正在重连…", "再接続中…")
# The terminal's covers and banners on the redesign (UI-DESIGN §6.3, R12):
# the reconnect capsule, the size follower's card, the multiplexer's empty
# state, the one banner and the copied link's chip.
add("Waiting for network…", "等待网络…", "ネットワークを待っています…")
add("No UDP reply from mosh-server.", "收不到 mosh-server 的 UDP 回包。", "mosh-server から UDP の応答がありません。")
add("Use SSH", "改用 SSH", "SSH を使う")
add("Network changed. mosh is reconnecting.", "网络换了，mosh 正在接上。", "ネットワークが変わりました。mosh が再接続しています。")
add("%@ isn't installed here, so this is a plain shell.", "这台主机没装 %@，现在是普通 shell。", "このホストには %@ がないため、通常のシェルです。")
add("mosh-server isn't installed here, so this is SSH.", "这台主机没装 mosh-server，现在走 SSH。", "このホストには mosh-server がないため、SSH で接続しています。")
add("Install…", "安装…", "インストール…")
add("This window is sized for %@.", "这个窗口按 %@ 的尺寸显示。", "このウインドウは %@ のサイズで表示されています。")
add("Use this %@'s size", "改用这台 %@ 的尺寸", "この %@ のサイズにする")
add("Use this device's size", "改用这台设备的尺寸", "このデバイスのサイズにする")
# B587: another client (a desktop, another phone) picked another pane in the
# window shown; the phone keeps its own until the person takes it back (B589:
# not "the desktop", which another phone is too).
add("Another device switched to a different pane.", "另一台设备切到了别的窗格。", "別のデバイスで別のペインに切り替わりました。")
add("Take it back", "拿回来", "こちらに戻す")
add("No %@ %@ on %@", "%3$@ 上没有 %1$@ %2$@", "%3$@ に %1$@ %2$@ がありません")
add("%@ isn't installed on this host", "这台主机没装 %@", "このホストには %@ がありません")
add("Open %@", "打开 %@", "%@ を開く")
add("Rename", "重命名", "名前を変更")
add("Run in terminal", "在终端运行", "ターミナルで実行")
add("SSH session is closed", "SSH 会话已关闭", "SSH セッションは閉じられています")
add("Show keyboard", "显示键盘", "キーボードを表示")
add("Small")
add("Still not detected. Run the command, wait for it to finish, then re-check.", "还是没检测到。运行命令，等它跑完，再重新检测。", "まだ検出されません。コマンドを実行し、完了を待ってから再チェックしてください。")
add("The server rejected this sign-in method. Try a different one (e.g. a key instead of a password).", "服务器不接受这种登录方式。换一种试试，比如用密钥代替密码。", "サーバーがこのサインイン方法を拒否しました。別の方法（例：パスワードの代わりに鍵）をお試しください。")
# keyboard-interactive asking what only the person can answer (B393)
add("The server rejected this sign-in method. Try a different one (e.g. a key instead of a password). It asks its own questions when you sign in, such as a one-time code, which Moshpit can't answer yet.",
    "服务器不接受这种登录方式。换一种试试，比如用密钥代替密码。这台服务器登录时会另外提问（比如一次性验证码），Moshpit 暂时还答不了。",
    "サーバーがこのサインイン方法を拒否しました。別の方法（例：パスワードの代わりに鍵）をお試しください。このサーバーはサインイン時にワンタイムコードなど独自の質問をしますが、Moshpit はまだ答えられません。")
add("Tiny")
add("done", "已完成", "完了")
add("needs you", "在等你", "要対応")
add("offline", "离线", "オフライン")
add("tmux + mosh aren't installed on this host.", "这台主机上没装 tmux 和 mosh。", "このホストには tmux と mosh がインストールされていません。")

# ---------- Rust core sessions (W10b): failures worded by the app (B202) ----------
add("mosh-server didn't start within %lld seconds. A startup file on the host may be waiting for input.",
    "mosh-server 在 %lld 秒内没有启动。主机上的某个启动脚本可能在等输入。",
    "mosh-server が %lld 秒以内に起動しませんでした。ホストの起動スクリプトが入力を待っている可能性があります。")
add("mosh-server didn't start: it never said where to reach it.",
    "mosh-server 没有启动：它一直没说该从哪里连它。",
    "mosh-server が起動しませんでした。接続先を知らせてきませんでした。")
add("The connection dropped while it was being set up. Try again.",
    "连接还在建立时就断了。再试一次。",
    "接続の確立中に切断されました。もう一度お試しください。")
add("%@ keys can't be used to sign in. Pick another key for this connection.",
    "%@ 密钥不能用来登录。给这个连接换一把密钥。",
    "%@ キーではサインインできません。この接続には別のキーを選んでください。")
add("This device is locked, so Moshpit can't read the saved password or key yet. It connects again once you unlock it.",
    "设备还锁着，Moshpit 暂时读不到保存的密码或密钥。解锁后会自己重新连接。",
    "デバイスがロックされているため、保存したパスワードやキーをまだ読み出せません。ロックを解除すると自動で接続し直します。")
add("The host stopped answering while the worktree was being removed.",
    "删除 worktree 的过程中主机不再回应了。",
    "worktree の削除中にホストが応答しなくなりました。")

# ---------- Side features on the Rust session's own connection (W14b) ----------
# HostConnectionError: the preview card, the folder sheet and the image overlay.
add("Moshpit is reconnecting to the host. Try again in a moment.",
    "Moshpit 正在重新连接主机，稍后再试。",
    "Moshpit がホストに再接続しています。少し待ってからもう一度お試しください。")
add("This session isn't connected to the host. Reconnect, then try again.",
    "这个会话没有连着主机。先重新连接，再试一次。",
    "このセッションはホストに接続していません。再接続してからもう一度お試しください。")
add("The host isn't answering on this session's connection. Try again in a moment.",
    "主机在这个会话的连接上没有回应，稍后再试。",
    "このセッションの接続でホストが応答していません。少し待ってからもう一度お試しください。")
add("The host doesn't let this account read it.",
    "主机不允许这个账户读取它。",
    "ホストがこのアカウントでの読み取りを許可していません。")


# ---------- Settings: the hub and its pages (UI-DESIGN §6.4, R8) ----------
# The hub fits on one screen; each page's footnote is at most two lines.
add("%lld pt", SAME)
add("Appearance", "外观", "外観")
# Settings › Appearance's choice (R18): the system's appearance.
add("System", "跟随系统", "システム")
add("On", "开", "オン")
add("Off", "关", "オフ")
add("SSH keys", "SSH 密钥", "SSH キー")
add("Set up a host", "配置主机", "ホストを設定")
# Host Setup names its host; Settings lists the hosts to pick from.
add("Set up %@", "配置 %@", "%@ を設定")
add("Connected", "已连接", "接続済み")
add("Not connected", "未连接", "未接続")
add("%@ isn't connected", "%@ 还没连上", "%@ に接続していません")
add("Setup runs on the host over a live session. Connect first, then set it up here.", "配置要通过连着的会话在主机上进行。先连上，再回来配置。", "設定は接続中のセッションを通してホスト上で行います。先に接続してから、ここで設定してください。")
add("Send feedback", "反馈问题", "フィードバックを送る")
add("Version", "版本", "バージョン")
add("The pit never closes.", "机器不眠。", "ピットは閉じない。")
add("The accent colors buttons and highlights; the terminal's colors are under Terminal.", "强调色给按钮和高亮上色；终端配色在「终端」里。", "アクセントカラーはボタンとハイライトに使われます。ターミナルの配色は「ターミナル」にあります。")
add("Size", "字号", "サイズ")
add("Smaller", "缩小", "小さく")
add("Larger", "放大", "大きく")
add("Trail while predicting", "预测时显示轨迹", "予測中に軌跡を表示")
add("mosh only", "仅 mosh", "mosh のみ")
add("Cursor settings apply to every session.", "光标设置对所有会话生效。", "カーソルの設定はすべてのセッションに適用されます。")
add("Keyboard on open", "打开终端时弹出键盘", "開いたらキーボードを表示")
add("Remote clipboard read", "允许远端读剪贴板", "リモートからのクリップボード読み取り")
add("OSC 52; off answers with an empty clipboard", "OSC 52；关闭时回给远端空内容", "OSC 52。オフなら空のクリップボードを返します")
add("Mosh for new connections", "新连接默认用 mosh", "新しい接続で mosh を使う")
add("Predictive echo", "预测回显", "予測エコー")
add("UDP ports", "UDP 端口", "UDP ポート")
add("Open the server's UDP ports in this range.", "服务器要放开这段 UDP 端口。", "サーバーでこの範囲の UDP ポートを開けてください。")
add("Forever", "一直", "ずっと")
add("Images upload to the host's ~/.moshpit/uploads; each connect clears expired ones.", "图片上传到主机的 ~/.moshpit/uploads，每次连接时清掉过期的。", "画像はホストの ~/.moshpit/uploads にアップロードされ、接続のたびに期限切れのものを削除します。")
add("Status in the Dynamic Island", "在灵动岛显示状态", "Dynamic Island に状況を表示")
# B741: what iOS itself has off of the app's alerts, on Settings › Notifications.
add("Off in iOS", "iOS 已关闭", "iOS でオフ")
add("Allow in iOS Settings", "去 iOS 设置里允许", "iOS の設定で許可")
add("iOS has notifications and Live Activities off for Moshpit.", "iOS 关掉了 Moshpit 的通知和实时活动。", "iOS で Moshpit の通知とライブアクティビティがオフになっています。")
add("iOS has notifications off for Moshpit, so no alert reaches you.", "iOS 关掉了 Moshpit 的通知，什么提醒都收不到。", "iOS で Moshpit の通知がオフになっているため、お知らせが届きません。")
add("iOS has Live Activities off for Moshpit, so the Dynamic Island stays empty.", "iOS 关掉了 Moshpit 的实时活动，灵动岛上什么也不会显示。", "iOS で Moshpit のライブアクティビティがオフになっているため、Dynamic Island に何も表示されません。")
add("Off keeps it private", "关掉就不显示具体内容", "オフにすると内容を表示しません")
add("Adds a mic key to the shortcut bar", "快捷键栏上多一个麦克风键", "ショートカットバーにマイクキーを追加")
add("Speech is transcribed on this device and never leaves it.", "语音只在本机转写，不会离开这台设备。", "音声はこの端末で書き起こされ、外に出ることはありません。")

# ---------- The companion pet (ui/src/home/pet.rs, ui/src/settings.rs) ----------
add("Tap to pet", "轻点摸摸它", "タップでなでる")
add("An egg. Tap it three times to hatch it; it also hatches by itself when an agent finishes a task.", "一枚蛋。点三下就能孵化，智能体完成任务时它也会自己破壳。", "たまご。3回タップするとかえります。エージェントがタスクを終えたときにも、ひとりでにかえります。")
add("Tap three times to hatch", "点三下孵化", "3回タップでかえる")
# The icon card after the hatch or a new look (B668): the owner's words.
add("Make %@ the app icon?", "把 App 图标换成%@吗？", "アプリのアイコンを%@にしますか？")
add("Change", "换", "変える")
add("Not Yet", "先不换", "まだ変えない")
# The pet talks (PET-DESIGN §17): its lines, the switch, the announcement.
add("Hm?", "嗯？", "ん？")
add("I'm here.", "在呢。", "いるよ。")
add("That tickles.", "好痒。", "くすぐったい。")
add("Still watching.", "我一直盯着呢。", "ちゃんと見てるよ。")
add("No rush.", "不急，慢慢来。", "あわてない、あわてない。")
add("Oh hi! Busy day?", "哟，今天挺忙啊？", "やあ！ 今日は忙しい？")
add("All in order.", "一切正常，放心。", "異常なし。")
add("We've got this!", "稳了！", "いけるいける！")
add("Drifting a little…", "有点飘……", "ちょっとふわふわ……")
add("The night is young.", "夜还长着呢。", "夜はこれから。")
add("%@ is on it.", "%@ 开工了。", "%@ が動きはじめた。")
add("Off it goes.", "跑起来了。", "走り出した。")
add("I'll keep watch.", "我帮你看着。", "見張っておくね。")
add("%@ needs you.", "%@ 在等你。", "%@ が待ってるよ。")
add("Your turn.", "该你了。", "きみの番だよ。")
add("It has a question.", "它有事问你。", "質問があるみたい。")
add("Done in %lld min.", "跑了 %lld 分钟，搞定。", "%lld 分で完了。")
add("%@ finished.", "%@ 干完了。", "%@ が終わったよ。")
add("That's a wrap.", "收工。", "おつかれさま。")
add("That was quick.", "这么快就好了。", "早かったね。")
add("Something broke over there.", "那边出问题了。", "向こうで何か起きたみたい。")
add("%@ isn't answering.", "%@ 没回应。", "%@ から返事がない。")
add("Lost %@.", "和 %@ 断开了。", "%@ との接続が切れた。")
add("I'll wait for it.", "等它回来。", "戻ってくるまで待つよ。")
add("Nap time.", "我先眯一会儿。", "ちょっと昼寝。")
add("All quiet.", "都歇着呢。", "みんな休んでる。")
add("Still up?", "还不睡？", "まだ起きてるの？")
add("Late one tonight.", "这么晚还在忙。", "今夜は遅いね。")
add("Morning.", "早。", "おはよう。")
add("New day. Let's go.", "新的一天，开工。", "新しい一日。さあ、始めよう。")
add("Hi. I'm %@.", "你好，我是%@。", "はじめまして、%@ だよ。")
add("I grew a little.", "我长大了一点。", "ちょっと大きくなった。")
add("100 tasks together.", "我们一起做完 100 个任务了。", "いっしょに 100 個のタスクを終えたね。")
add("New network. Still here.", "换网了，我还在。", "ネットが変わった。まだいるよ。")
add("A thousand. Shiny.", "一千个了，闪闪发光。", "1000 個目。ピカピカ。")
add("Yes, boss.", "遵命。", "かしこまり。")
add("Mosh!", "蹦起来！", "跳ねろ！")
add("Small hours. My time.", "凌晨，我的主场。", "真夜中はぼくの時間。")
add("Let It Talk", "让它说话", "おしゃべりさせる")
add("A short line on Home now and then", "在首页偶尔说一句", "ホームでときどきひとこと")
add("%@: %@", "%@：%@", "%@：%@")
add("Tun", "墩墩", "トゥン")
add("Coo", "咕咕", "クー")
add("Dig", "挖挖", "ディグ")
add("Hex", "六六", "ヘックス")
add("Ether", "漂漂", "イーサ")
add("Lumen", "扑扑", "ルーメン")
add("Tardigrade", "水熊虫", "クマムシ")
add("Pigeon", "信鸽", "ハト")
add("Mole", "鼹鼠", "モグラ")
add("Axolotl", "六角恐龙", "ウーパールーパー")
add("Jellyfish", "水母", "クラゲ")
add("Moth", "夜蛾", "ガ")
add("Companion", "伙伴", "コンパニオン")
add("COMPANION", "伙伴", "コンパニオン")
add("Pixel Companion", "像素伙伴", "ピクセルのコンパニオン")
add("On the Dynamic Island, the lock screen and Home", "在灵动岛、锁屏和首页陪着你", "Dynamic Island、ロック画面、ホームでそばにいます")
add("Egg", "蛋", "たまご")
add("Tap it three times on Home to hatch it. It also hatches by itself when an agent finishes a task.", "在首页点它三下就能孵化，智能体完成任务时它也会自己破壳。", "ホームで3回タップするとかえります。エージェントがタスクを終えたときにも、ひとりでにかえります。")
add("Tap it on Home and it reacts. On the Dynamic Island and the lock screen it changes pose but doesn't move. Each server's mark becomes a pixel creature too.", "在首页轻点它会有回应。灵动岛和锁屏上只换姿势，不放动画。每台服务器的头像也会变成像素小怪。", "ホームでタップすると反応します。Dynamic Island とロック画面ではポーズが変わるだけで、動きません。サーバーのアイコンも、それぞれピクセルの生き物になります。")
add("Hatchling", "幼崽", "あかちゃん")
add("Grown", "成年", "おとな")
add("Seasoned", "资深", "ベテラン")
add("Natural", "原色", "ナチュラル")
add("Peach", "蜜桃", "ピーチ")
add("Mint", "薄荷", "ミント")
add("Sky", "天蓝", "スカイ")
add("Lilac", "丁香", "ライラック")
add("Sand", "沙色", "サンド")
add("Slate", "石板", "スレート")
add("Rose", "玫瑰", "ローズ")
add("Moss", "苔绿", "モス")
add("Charcoal", "炭灰", "チャコール")
add("Aurora", "极光", "オーロラ")
add("Plain", "素色", "無地")
add("Spots", "斑点", "ぶち")
add("Stripes", "条纹", "しま")
add("Belly Patch", "肚皮", "おなか")
add("Round", "圆眼", "まる目")
add("Sleepy", "眯眼", "ねむ目")
add("Starry", "星星眼", "キラキラ目")
add("Button", "豆豆眼", "つぶら目")
add("Short", "小个子", "こがら")
add("Standard", "标准", "ふつう")
add("Tall", "高个子", "のっぽ")
add("a sprout", "一棵嫩芽", "新芽")
add("a tuft of moss", "一撮苔藓", "こけ")
add("a crystal", "一颗小晶体", "小さな結晶")
add("freckles", "雀斑", "そばかす")
add("a star", "一颗星斑", "星のもよう")
add("New fur: %@", "毛色换成了%@", "毛色：%@")
add("New pattern: %@", "身上长出了%@", "もよう：%@")
add("Grew %@", "长出了%@", "%@が生えた")
add("New eyes: %@", "眼睛变成了%@", "目：%@")
add("Hatched at three in the morning", "凌晨三点孵出来的", "午前3時にかえった")
add("Made it through a network change", "换网没掉线", "ネットワークが変わっても切れなかった")
add("A thousand tasks together", "一起完成了一千个任务", "一緒に1,000件のタスク")
add("Named sudo", "叫 sudo", "名前は sudo")
add("Shiny", "闪光", "キラキラ")
add("Nickname", "名字", "名前")
add("GROWING", "成长", "成長")
add("Tasks together", "一起完成的任务", "一緒に終えたタスク")
add("Days together", "相处天数", "一緒にいた日数")
add("Next: %@", "下一步：%@", "次は%@")
add("Fully grown", "已经长到最大", "いちばん大きくなった")
add("It grows next time you see it", "下次见到它时就会长大", "次に会うとき大きくなります")
add("TRAITS", "特征", "特徴")
add("Fur", "毛色", "毛色")
add("Pattern", "花纹", "もよう")
add("Eyes", "眼睛", "目")
add("Build", "个头", "体格")
add("Grew up: %@", "长大了：%@", "%@になった")
add("Hatched", "孵化", "誕生")
add("Today", "今天", "今日")
add("CHANGES", "变化", "変化")
add("It's yours alone. iCloud keeps it the same on your iPhone and iPad.", "它只属于你，跟着 iCloud 在你的 iPhone 和 iPad 之间同步。", "あなただけの子です。iCloud で iPhone と iPad に同じ子がいます。")
plural("%lld more tasks", "%lld more task", "还差 %lld 个任务", "あと %lld 件")
plural("%lld more days", "%lld more day", "还要 %lld 天", "あと %lld 日")
plural("%lld days ago", "%lld day ago", "%lld 天前", "%lld 日前")

# ---------- Vibe Island + widgets (the MoshpitIsland extension) ----------
# Text keys and String(localized:) resolve against Bundle.main, which inside the
# widget extension is the EXTENSION — every string the Live Activity, Dynamic
# Island and home/lock-screen widgets show needs a key in this table.
ISLAND = {}
def island(key, zh, ja):
    ISLAND[key] = (zh, ja)
    S[key] = (zh, ja)
island("working", "工作中", "作業中")
island("idle", "空闲", "アイドル")
# AgentState.label (MoshpitKit) is the other two.
for k in ["needs you", "done"]:
    ISLAND[k] = S[k]
ISLAND["Moshpit"] = SAME
# Header summaries ("1 needs you · 2 working · 1 done") and widget copy.
ISLAND["%lld needs you"] = plural_spec("%lld needs you", "%lld needs you",
                                       "%lld 个在等你", "対応待ち %lld", en_other="%lld need you")
ISLAND["%lld working"] = ("%lld 个工作中", "作業中 %lld")
# A dropped connection's agents (B662): the rows' word and the header's count.
ISLAND["offline"] = S["offline"]
ISLAND["%lld offline"] = ("%lld 个离线", "オフライン %lld")
ISLAND["%lld done"] = plural_spec("%lld done", "%lld done", "%lld 个已完成", "完了 %lld")
ISLAND["+%lld more"] = ("还有 %lld 个", "ほか %lld 件")
ISLAND["paused — open Moshpit to refresh"] = ("已暂停，打开 Moshpit 刷新", "一時停止中 — Moshpit を開くと更新されます")
ISLAND["paused — open Moshpit"] = ("已暂停，打开 Moshpit", "一時停止中 — Moshpit を開く")
ISLAND["no active agents"] = ("没有智能体在跑", "稼働中のエージェントなし")
ISLAND["Moshpit · no agents"] = ("Moshpit · 没有智能体", "Moshpit · エージェントなし")
ISLAND["Agent Status"] = ("智能体状态", "エージェントの状況")
ISLAND["Live status of your Moshpit agents."] = ("实时查看 Moshpit 里各个智能体在做什么。", "Moshpit のエージェントの状況をリアルタイムで表示します。")
# Circular lock-screen widget: one tiny word under the count.
ISLAND["wait"] = ("等你", "待ち")
ISLAND["agt"] = ("智能体", "体")

# ---------- Share sheet (the MoshpitShare extension) ----------
# Same Bundle.main rule as the widget: the share extension has its own catalog.
SHARE = {}
for k in ["working", "needs you", "done", "idle", "Cancel"]:
    SHARE[k] = S[k]
SHARE["Send to agent"] = ("发给智能体", "エージェントに送る")
SHARE["1 image — pick the agent that should see it"] = (
    "1 张图片，选一个要看它的智能体",
    "画像 1 枚 — 見せるエージェントを選んでください")
SHARE["%lld images — pick the agent that should see them"] = (
    "%lld 张图片，选一个要看它们的智能体",
    "画像 %lld 枚 — 見せるエージェントを選んでください")
SHARE["Agent list is from %lld min ago — targets may have moved on. Queued images are delivered when the session is next live."] = (
    "这份智能体列表是 %lld 分钟前的，对方可能已经换了地方。排队的图片会在会话下次连上时送到。",
    "このエージェント一覧は %lld 分前のものなので、状況が変わっているかもしれません。キューに入れた画像は、次にセッションがつながったときに届きます。")
SHARE["Delivered as an uploaded path the moment the session is live — instantly, if Moshpit is connected right now."] = (
    "会话一连上，图片就会上传，并把路径交给智能体；Moshpit 现在正连着的话，马上就到。",
    "セッションがつながった時点でアップロードし、そのパスを渡します。いま Moshpit が接続中なら、すぐに届きます。")
SHARE["No agents to send to"] = ("没有可以发送的智能体", "送信先のエージェントがありません")
SHARE["Open Moshpit and connect to a session with a running agent first — the share sheet lists every agent Moshpit is watching."] = (
    "先打开 Moshpit，连上一个有智能体在跑的会话。这里会列出 Moshpit 正在留意的每个智能体。",
    "まず Moshpit を開き、エージェントが動いているセッションに接続してください。共有シートには Moshpit が見守っているすべてのエージェントが並びます。")
SHARE["Couldn't queue the images"] = ("图片没能排进队列", "画像をキューに入れられませんでした")
SHARE["Queued for %@"] = ("已排队，等着发给 %@", "%@ 宛てにキューに入れました")
SHARE["Moshpit uploads and hands over the path the moment that session is live."] = (
    "那个会话一连上，Moshpit 就会上传图片并把路径交过去。",
    "そのセッションがつながった時点で、Moshpit がアップロードしてパスを渡します。")

# ---------- Agent notifications (shared with the push extension) ----------
# AgentNotificationCopy (MoshpitKit) composes every agent card — the local one
# the app posts and the pushed one the notification service extension writes.
# String(localized:) resolves against Bundle.main, which inside the extension is
# the EXTENSION, so it carries these keys in a catalog of its own.
PUSH = {}
def push(key, zh, ja):
    PUSH[key] = (zh, ja)
    S[key] = (zh, ja)
push("✓ %@ finished", "✓ %@ 已完成", "✓ %@ が完了しました")
push("%@ needs you", "%@ 在等你", "%@ が待っています")


# ---------- Info.plist permission texts ----------
# The English lives in project.yml (XcodeGen regenerates Info.plist from it);
# it is read from there so the two can never drift. zh-Hans / ja sit here like
# every other string; the other locales come from scripts/gen/locales/*.json
# under the plist key name.
INFOPLIST_ZH_JA = {
    "NSFaceIDUsageDescription": (
        "用 Face ID 解锁 SSH 密钥",
        "Face ID で SSH キーをロック解除します"),
    "NSMicrophoneUsageDescription": (
        "语音输入要用麦克风，这样你可以把命令和提示词直接说进终端。音频在这台设备上转写，用 Apple 的语音模型或你下载的语音模型，不会上传到任何地方。",
        "音声入力はマイクを使い、コマンドやプロンプトをターミナルに向けて話せるようにします。音声はこの端末上で、Apple の音声モデルまたはダウンロードした音声モデルによって書き起こされ、どこにもアップロードされません。"),
    "NSSpeechRecognitionUsageDescription": (
        "听写在本机做语音识别。你的语音和转写文字都不会离开这台设备，Moshpit 不使用服务器端识别。",
        "音声入力は端末内で音声認識を行います。音声も書き起こしもこの端末を離れず、Moshpit はサーバー側の認識を使いません。"),
    "NSLocalNetworkUsageDescription": (
        "Moshpit 通过 SSH 和 mosh（UDP）连接你局域网里的服务器。没有这个权限，连 192.168.x.x 这类内网主机会失败。",
        "Moshpit は SSH と mosh（UDP）でローカルネットワーク上のサーバーに接続します。この許可がないと、192.168.x.x のような LAN 上のホストへの接続は失敗します。"),
    "NSCameraUsageDescription": (
        "相机用来拍点东西，比如白板、草图、屏幕，然后附到终端会话里。照片会在这台设备上去掉位置信息，只上传到你自己的服务器。",
        "カメラでホワイトボードやスケッチ、画面などを撮影し、ターミナルセッションに添付できます。写真はこの端末上で位置情報を取り除き、あなた自身のサーバーにのみアップロードされます。"),
}


def infoplist_english():
    """Pull the permission texts out of project.yml, keyed by plist key."""
    text = open(os.path.join(root, "project.yml"), encoding="utf-8").read()
    found = {}
    for key, value in re.findall(r'^\s+(NS\w+UsageDescription): "(.*)"\s*$', text, re.M):
        found.setdefault(key, value)
    missing = set(INFOPLIST_ZH_JA) - set(found)
    if missing:
        raise SystemExit(f"project.yml lacks {sorted(missing)}; keep INFOPLIST_ZH_JA in step with it")
    return found


# ---------- Additional locales ----------
# One JSON file per locale in scripts/gen/locales/, mapping the English key to
# its translation. Plural keys (see plural() above) take {"one": …, "other": …}
# in languages that inflect, or a single string where they do not. SAME keys
# are filled in automatically and must not appear in the files. Style notes and
# the per-language glossary: scripts/gen/locales/STYLE.md.
LOCALES = ["zh-Hant", "ko", "de", "es", "fr", "pt-BR"]
PLURAL_LOCALES = {"de", "es", "fr", "pt-BR"}
LOCALE_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "locales")

SPEC = re.compile(r"%(?:\d+\$)?(?:lld|llu|ld|lu|d|u|f|@|%)")


def specifiers(text):
    """Multiset of format specifier types, positions stripped — '%2$@' == '%@'."""
    return sorted(re.sub(r"\d+\$", "", m) for m in SPEC.findall(text))


def unique_keys(path):
    """An object_pairs_hook that refuses a key given twice: json.load keeps
    the last silently, so a second translation of a string would replace
    the first without anyone choosing it."""
    def hook(pairs):
        table = {}
        for key, value in pairs:
            if key in table:
                raise SystemExit(f"{path}: {key!r} is given twice; keep one")
            table[key] = value
        return table
    return hook


def load_locales():
    tables = {}
    for loc in LOCALES:
        path = os.path.join(LOCALE_DIR, loc + ".json")
        if not os.path.exists(path):
            tables[loc] = {}
            continue
        with open(path, encoding="utf-8") as f:
            try:
                tables[loc] = json.load(f, object_pairs_hook=unique_keys(path))
            except json.JSONDecodeError as exc:
                raise SystemExit(f"{path}: {exc}")
    return tables


LOCALE_TABLES = load_locales()
PROBLEMS = []   # (locale, key, what)
MISSING = {loc: [] for loc in LOCALES}


def locale_value(loc, key, spec):
    """Translation for one key in one extra locale, or None when the file lacks it."""
    # SAME keys (brand names, glyphs, bare format strings) fall back to the
    # key itself, but a locale file may still override one — "%@ %@" is
    # killVerb + noun, and German or Korean want the noun first.
    if spec is SAME and key not in LOCALE_TABLES[loc]:
        return key
    value = LOCALE_TABLES[loc].get(key)
    if value is None:
        MISSING[loc].append(key)
        return None
    plural_spec = isinstance(spec, dict) and spec.get("plural")
    if plural_spec and loc in PLURAL_LOCALES:
        if not (isinstance(value, dict) and {"one", "other"} <= set(value)):
            PROBLEMS.append((loc, key, "plural key needs {\"one\": …, \"other\": …}"))
            return None
        for form in ("one", "other"):
            if specifiers(value[form]) != specifiers(key):
                PROBLEMS.append((loc, key, f"format specifiers differ in plural '{form}'"))
        return value
    if isinstance(value, dict):
        PROBLEMS.append((loc, key, "expected a plain string"))
        return None
    if specifiers(value) != specifiers(key):
        PROBLEMS.append((loc, key, f"format specifiers {specifiers(key)} vs {specifiers(value)}"))
    return value


def unit(value):
    return {"stringUnit": {"state": "translated", "value": value}}


def plural_unit(forms):
    return {"variations": {"plural": {
        form: {"stringUnit": {"state": "translated", "value": text}} for form, text in forms.items()}}}


def entry(key, spec, english=None):
    locs = {}
    if english is not None:
        locs["en"] = unit(english)
    if spec is SAME:
        locs["zh-Hans"] = unit(key)
        locs["ja"] = unit(key)
    elif isinstance(spec, dict) and spec.get("plural"):
        locs["en"] = plural_unit({"one": spec["en_one"], "other": spec.get("en_other", key)})
        locs["zh-Hans"] = unit(spec["zh"])
        locs["ja"] = unit(spec["ja"])
    else:
        zh, ja = spec
        locs["zh-Hans"] = unit(zh)
        locs["ja"] = unit(ja)
    for loc in LOCALES:
        value = locale_value(loc, key, spec)
        if value is None:
            continue
        locs[loc] = plural_unit(value) if isinstance(value, dict) else unit(value)
    return {"localizations": locs}


def write_catalog(path, table, english=None):
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
        merged[key] = entry(key, spec, (english or {}).get(key))

    catalog = {"sourceLanguage": "en", "strings": merged, "version": "1.0"}
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(catalog, f, ensure_ascii=False, indent=2, sort_keys=True)
        f.write("\n")
    kept = len(merged) - len(table)
    print(f"{path}: {len(table)} curated + {kept} kept = {len(merged)} keys")


def report_locales(all_keys):
    """Coverage per extra locale, unknown keys in the files, specifier problems."""
    ok = True
    translatable = [k for k, spec in all_keys.items() if spec is not SAME]
    for loc in LOCALES:
        unknown = sorted(set(LOCALE_TABLES[loc]) - set(all_keys))
        missing = sorted(set(MISSING[loc]))
        done = len(translatable) - len(missing)
        line = f"  {loc:8} {done}/{len(translatable)}"
        if missing:
            line += f"  missing {len(missing)}"
        if unknown:
            line += f"  UNKNOWN KEYS {len(unknown)}: " + "; ".join(repr(u[:50]) for u in unknown[:5])
            ok = False
        print(line)
        if "--missing" in sys.argv:
            for k in missing:
                print(f"      - {k!r}")
    for loc, key, what in PROBLEMS:
        print(f"  {loc}: {what}: {key!r}")
        ok = False
    return ok


def translated(loc):
    """Does this catalog entry carry a real translation for a language?"""
    unit_ = loc.get("stringUnit") or {}
    if unit_.get("value"):
        return True
    # Plurals keep their values one level down, under variations.
    return bool(loc.get("variations"))


def check(paths):
    """Report every string the app needs that has no translation in some language.

    Reads the keys the compiler actually extracted (`.stringsdata` emitted by
    SWIFT_EMIT_LOC_STRINGS) rather than grepping the source, so interpolation
    is already normalized to %@ / %lld and nothing is missed by a regex that
    didn't anticipate a call shape. Pass --derived-data <dir> when the build
    did not go to the default DerivedData location.
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

    derived = os.path.expanduser("~/Library/Developer/Xcode/DerivedData/Moshpit-*")
    if "--derived-data" in sys.argv:
        derived = sys.argv[sys.argv.index("--derived-data") + 1]
    needed = {}
    pattern = os.path.join(derived, "Build/Intermediates.noindex/*/Debug-*/*/Objects-normal/*/*.stringsdata")
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

    languages = ["zh-Hans", "ja"] + LOCALES
    gaps = []
    for key, source in sorted(needed.items()):
        locs = (have.get(key) or {}).get("localizations") or {}
        missing = [lang for lang in languages if not translated(locs.get(lang) or {})]
        if missing:
            gaps.append((key, source, missing))

    for key, source, missing in gaps:
        print(f"{source}: {'/'.join(missing)}\t{key}")
    print(f"\n{len(needed)} keys used, {len(gaps)} without a full translation")
    return 1 if gaps else 0


root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CATALOGS = [os.path.join(root, "Moshpit/Resources/Localizable.xcstrings"),
            os.path.join(root, "Extensions/MoshpitIsland/Localizable.xcstrings"),
            os.path.join(root, "Extensions/MoshpitPush/Localizable.xcstrings"),
            os.path.join(root, "Extensions/MoshpitShare/Localizable.xcstrings")]
INFOPLIST_CATALOG = os.path.join(root, "Moshpit/Resources/InfoPlist.xcstrings")

if "--check" in sys.argv:
    raise SystemExit(check(CATALOGS + [INFOPLIST_CATALOG]))

write_catalog(CATALOGS[0], S)
write_catalog(CATALOGS[1], ISLAND)
write_catalog(CATALOGS[2], PUSH)
write_catalog(CATALOGS[3], SHARE)
INFOPLIST_EN = infoplist_english()
write_catalog(INFOPLIST_CATALOG, {k: INFOPLIST_ZH_JA[k] for k in INFOPLIST_ZH_JA}, english=INFOPLIST_EN)

ALL = dict(S); ALL.update(ISLAND); ALL.update(PUSH); ALL.update(SHARE); ALL.update({k: INFOPLIST_ZH_JA[k] for k in INFOPLIST_ZH_JA})
print("extra locales (scripts/gen/locales/*.json):")
if not report_locales(ALL):
    raise SystemExit("locale files have problems; see above")
