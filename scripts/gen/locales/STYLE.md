# Locale files: how the copy is written

One file per language, `<locale>.json`, mapping the English key (exactly as it
appears in `gen_xcstrings.py`) to the translation. Plural keys take
`{"one": …, "other": …}` in languages that inflect (de, es, fr, pt-BR) and a
single string elsewhere. Brand names, protocol words and glyphs are `SAME` in
the generator and may be left out; a file may still override one when word
order differs (de/ko/zh-Hant reorder `"%@ %@"`, killVerb + noun, to put the
noun first). `python3 scripts/gen/gen_xcstrings.py`
regenerates the catalogs and prints coverage per locale; `--missing` lists the
keys a file still lacks; `--check --derived-data <dir>` compares against what
the compiler actually extracted.

The same rules that govern the Chinese copy apply everywhere: say what the
English says, the way a native speaker of that language would say it in an
app. No word-for-word rendering, no English rhetoric carried over, no
sentence that lets the reader reconstruct the English. Apple's own UI
vocabulary for that language wins when it exists (Settings, Lock Screen,
Dynamic Island, Live Activity, Keychain, Face ID, Wi-Fi, cellular…).
Product and tool names stay in English: Moshpit, tmux, herdr, mosh, SSH,
Claude Code, Codex, git worktree, hooks, Whisper, Hugging Face, PATH.
Format specifiers must match the English (positions may be reordered with
`%1$@`). Placeholders such as `%@` and `%lld` are filled at runtime — leave a
space around them where the language needs one.

## zh-Hant (台灣用語)

- Apple 台灣用語：設定、鎖定畫面、動態島、即時動態、鑰匙圈、安全隔離區、拷貝／貼上、
  行動網路、連接埠、伺服器、裝置、檔案、資料夾、鍵盤、快速鍵、工具列、字體、字級。
- tmux 詞彙：session＝工作階段、window＝視窗、pane＝窗格、tab＝分頁、workspace＝工作區。
- 密碼學：金鑰（不是密鑰）、公開金鑰、私密金鑰、指紋、密碼短語。
- agent 保留英文（台灣科技寫作慣用英文；「代理」會和代理伺服器混淆）。prompt＝提示詞。
  hooks 保留英文。relay＝中繼伺服器。
- 動作：點一下、長按、拖移、滑動。用「你」。全形標點，中英文之間空一格。

## ko

- Apple 한국어 UI 어투: 버튼·레이블은 명사형(「저장」, 「연결」), 설명문은 「~합니다.」,
  안내는 「~하세요.」. 존칭 없이 「~하세요」체.
- Apple 용어: 설정, 잠금 화면, Dynamic Island, 실시간 현황, 키체인, Face ID, Wi-Fi,
  셀룰러, 기기, 서체, 도구 막대, 단축키, 받아쓰기, 암호, 클립보드.
- tmux 어휘: 세션, 창, 패널(pane), 탭, 작업 공간. 에이전트, 프롬프트, 훅(hooks), 릴레이.
- 조사는 자연스럽게 붙이되 `%@`, `%lld` 뒤에는 「%@에」처럼 붙여 쓴다.

## de

- Duzen, wie Apple im deutschen iOS ("Tippe auf …", "Du kannst …").
- Apple-Begriffe: Einstellungen, Sperrbildschirm, Dynamic Island, Live-Aktivität,
  Schlüsselbund, Face ID, WLAN (nicht Wi-Fi), Mobilfunk, Mitteilungen, Sichern (Save),
  Einsetzen (Paste), Kurzbefehle, Symbolleiste, Zwischenablage, Gerät.
- tmux: Sitzung, Fenster, Panel, Tab, Arbeitsbereich. Agent, Prompt, Hooks, Relay.
- Schlüssel: öffentlicher/privater Schlüssel, Fingerabdruck, Passphrase, Host-Schlüssel.
- Zusammensetzungen mit Bindestrich (SSH-Schlüssel, UDP-Portbereich, Push-Relay).

## es

- Español neutro con «tú», sin vosotros ni formas exclusivamente peninsulares o
  rioplatenses; léxico que se entienda igual en México y en España.
- Términos Apple: Ajustes, pantalla bloqueada, Dynamic Island, Actividad en Vivo,
  llavero, Face ID, Wi-Fi, datos móviles, notificaciones, portapapeles, tipo de letra,
  barra de herramientas, atajos, dictado.
- tmux: sesión, ventana, panel, pestaña, espacio de trabajo. Agente, prompt, hooks,
  servidor relay. Claves: clave pública/privada, huella, frase de contraseña.
- Signos de apertura ¿¡ obligatorios; sin mayúsculas iniciales en cada palabra de un
  título salvo nombres propios.

## fr

- Vouvoiement, comme l'interface iOS en français.
- Termes Apple : Réglages, écran verrouillé, Dynamic Island, Activité en direct,
  trousseau, Face ID, Wi-Fi, données cellulaires, notifications, presse-papiers, police,
  barre d'outils, raccourcis, dictée, micro.
- tmux : session, fenêtre, panneau, onglet, espace de travail. Agent, prompt, hooks,
  relais. Clés : clé publique/privée, empreinte, phrase secrète, clé d'hôte.
- Espace insécable avant « : », « ; », « ? », « ! » ; guillemets « » ; apostrophe typographique.

## pt-BR

- «Você», tom direto do iOS brasileiro.
- Termos Apple: Ajustes, Tela Bloqueada, Dynamic Island, Atividade ao Vivo, Chaves
  (Keychain), Face ID, Wi-Fi, dados celulares, notificações, área de transferência,
  fonte, barra de ferramentas, atalhos, ditado, baixar (download), Apagar (delete).
- tmux: sessão, janela, painel, aba, espaço de trabalho. Agente, prompt, hooks, relay.
  Chaves: chave pública/privada, impressão digital, frase-senha, chave do host.
- Branch, worktree, shell e prompt ficam em inglês, como na comunidade dev brasileira.
