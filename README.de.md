<!-- source: README.md 2d4fb4a3e4f6551abe6e997954516d14aadde7dd -->
<div align="center">

<img src="Moshpit/Resources/AlternateIcons/Previews/IconPreview-Moshpit@3x.png" width="112" alt="App-Symbol von Moshpit: ein Pixel-Ei, das über ein Terminal-Panel lugt">

# Moshpit

### Das Terminal für Coding-Agents auf deinen eigenen Rechnern.

**Claude Code, Codex und Gemini arbeiten auf dem Server weiter. Braucht dich einer, erfährt es dein iPhone.**<br>
SSH, Mosh, tmux und herdr auf iPhone und iPad: Jede Frage kommt als verschlüsselte Push-Mitteilung,<br>
und eine Live-Aktivität zeigt, wer wartet, wer arbeitet und wie lange schon.

[![App Store](https://img.shields.io/badge/App_Store-iPhone_%26_iPad-111111?logo=apple&logoColor=white)](https://apps.apple.com/app/id6799896801) ![iOS and iPadOS 18+](https://img.shields.io/badge/iOS_%7C_iPadOS-18%2B-111111?logo=apple&logoColor=white) [![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-111111)](LICENSE)

**[App Store](https://apps.apple.com/app/id6799896801)** &nbsp;·&nbsp; **[Website (auf Englisch)](https://moshpit.cluas.eu.org)** &nbsp;·&nbsp; **[Dokumentation (auf Englisch)](https://moshpit.cluas.eu.org/docs)** &nbsp;·&nbsp; **[Issues](https://github.com/Cluas/moshpit/issues)**

[English](README.md) · [简体中文](README-zh.md) · [繁體中文](README.zh-Hant.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · **Deutsch** · [Español](README.es.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md)

<br>

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/de-DE/01-lock.png" width="250" alt="Wer dich braucht und wofür – direkt auf dem Sperrbildschirm."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/de-DE/03-home.png" width="250" alt="Wer arbeitet, wer wartet – auf einen Blick."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/de-DE/02-pane.png" width="250" alt="Antippen, und du bist im Panel, das gefragt hat.">

</div>

<br>

## Ist Moshpit das Richtige für dich?

Ja, wenn du Claude Code, Codex, Gemini CLI oder einen anderen Coding-Agent auf einem Rechner laufen lässt, den du per SSH erreichst (Workstation, Server, eine Maschine in der Cloud), oft in tmux oder herdr, und du dem Agent, der gerade fragt, lieber vom iPhone aus antwortest, statt zurück an den Laptop zu gehen:

- **Du merkst, wann du gebraucht wirst.** Eine Frage landet als Push-Mitteilung auf dem Sperrbildschirm, verschlüsselt auf deinem Host und erst auf dem iPhone entschlüsselt. Ein Tipp, und du bist in dem Panel, das gefragt hat.
- **Alle Agents auf einen Blick.** Die Übersicht zeigt über alle Hosts hinweg zuerst, wer auf dich wartet, dann, wer arbeitet und wie lange schon, und zuletzt, wer inaktiv ist.
- **Ein echtes Terminal.** Mosh, das auch bei Netzwechseln hält, tmux-Fenster und -Panels, die du nativ verwaltest, eine Kurzbefehlsleiste und Bilder direkt im Terminal.

Nein, wenn deine Agents auf keinem Rechner laufen, an den du herankommst: Moshpit ist ein Terminal. Agents laufen nicht darin, und es ist auch keine Chat-App für gehostete Agents. Moshpit gibt es nur für iPhone und iPad; die Android-App ist in Entwicklung und noch nicht veröffentlicht.

## Schnellstart

**1 · Installieren.** Lade Moshpit aus dem [App Store](https://apps.apple.com/app/id6799896801) (Einmalkauf) oder [bau es selbst](#aus-dem-quellcode-bauen).

**2 · Host hinzufügen.** Gib Adresse und Benutzer ein und melde dich mit Passwort oder SSH-Schlüssel an. Den Schlüssel kannst du auf dem iPhone erstellen, importieren oder als PEM einsetzen. Jeder SSH-Server funktioniert.

**3 · Host einrichten (optional).** *Host einrichten* installiert die Hooks, die den Status jedes Agents melden, und koppelt den Host für Push-Mitteilungen. Bevor es auf dem Host irgendetwas ändert, fragt es nach. Codex braucht dort noch einen Schritt: Führe in Codex `/hooks` aus und vertraue den vier Einträgen von Moshpit.

**4 · Mosh, tmux oder herdr dazunehmen (optional).** Mit Mosh überlebt die Sitzung Netzwechsel; tmux und herdr stellen ihre Panels nativ dar. Für jedes davon bringt die App eine Installationsanleitung mit.

## Unterstützte Agents

| Agent | Status | Wartet auf dich | Fertig |
| :-- | :-: | :-: | :-: |
| Claude Code | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ ¹ | ✓ |
| Gemini CLI | ✓ | ✓ ² | ✓ |
| Qwen Code | ✓ | ✓ | ✓ |
| Qoder | ✓ | ✓ | ✓ |
| Factory (`droid`) | ✓ | ✓ | ✓ |
| CodeBuddy | ✓ | ✓ | ✓ |

Der **Status** erscheint in der Übersicht, in der Dynamic Island und in der Live-Aktivität; **Wartet auf dich** und **Fertig** sind Push-Mitteilungen. ¹ Codex fragt über seine Berechtigungsanfragen. ² Gemini CLI fragt nur, wenn ein Tool eine Berechtigung braucht.

*Host einrichten* installiert die Hooks für die Agents, die es auf dem Host findet. Unter tmux stammt der Status in der Übersicht von diesen Hooks; unter herdr meldet herdr ihn selbst, ganz ohne Einrichtung. Die Push-Mitteilungen kommen immer von den Hooks, egal wo der Agent läuft: in tmux, in herdr oder in einer einfachen Shell.

Ein Push „Wartet auf dich“ geht erst nach 30 Sekunden raus. Eine Frage, die du am Schreibtisch beantwortest, kommt so gar nicht erst auf dem iPhone an. Ein Push „Fertig“ sagt, wie lange der Durchlauf gedauert hat, und, wenn auf dem Host `jq` installiert ist, worauf der Agent geantwortet hat. Hat der Durchlauf drei Minuten oder länger gedauert, gibt es einen Ton; kürzere kommen lautlos an.

## Wie alles zusammenhängt

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/architecture-dark.svg">
  <img src="docs/assets/architecture-light.svg" width="900" alt="So hängt Moshpit zusammen: Die App auf iPhone und iPad (eine dünne Swift-Schicht über einem Rust-Kern) erreicht deinen Host per SSH oder Mosh und liest die Panels von tmux oder herdr sowie den Status jedes Agents. Auf dem Host vermerken die Hooks der Agents an jedem Panel seinen Status und übergeben „Wartet auf dich“ und „Fertig“ an ein Skript, das sie verschlüsselt. Der verschlüsselte Push geht über ein Relay, das nur Chiffretext sieht, und über APNs an den Mitteilungsdienst auf dem iPhone, der ihn entschlüsselt.">
</picture>

Die App verbindet sich per SSH oder Mosh mit deinem Host und steuert tmux über dessen Control Mode und herdr über sein eigenes Protokoll. Deshalb werden Panels nativ gezeichnet, und der Status jedes Agents kommt als Daten an, statt aus seiner Ausgabe erraten zu werden. Auf dem Host rufen die Hooks jedes Agents ein kleines Skript auf. Es vermerkt den Status des Agents am Panel und verschlüsselt bei „Wartet auf dich“ und „Fertig“ eine Push-Mitteilung mit dem Schlüssel, der unter *Host einrichten* gekoppelt wurde. Das Relay reicht den verschlüsselten Umschlag an APNs weiter und sieht nur Chiffretext; der Mitteilungsdienst auf dem iPhone entschlüsselt ihn. Das Protokoll von Anfang bis Ende steht in [docs/PUSH.md](docs/PUSH.md) (auf Englisch).

## Was es kann

### Vor allem ein Terminal

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/de-DE/05-overview.png" width="220" alt="Alle tmux- und herdr-Panels auf einem Bildschirm."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/de-DE/08-keyboard.png" width="220" alt="Eine Tastatur fürs Terminal.">
</p>

| Funktion | Was dahintersteckt |
| :-- | :-- |
| **SSH und Mosh** | Mosh mit Vorhersage-Echo. Sitzungen überstehen den Wechsel zwischen WLAN und 5G, den Ruhezustand und schlechtes Hotel-WLAN. Hat eine Verbindung die Zeit im Hintergrund überlebt, macht die App direkt mit ihr weiter, ohne neu zu verbinden |
| **Sagt, woran die Verbindung scheitert** | Der Port hat die Verbindung abgelehnt, der Name ließ sich nicht auflösen (oder die Berechtigung „Lokales Netzwerk“ ist aus), der Host hat nicht geantwortet (prüf dein VPN), oder aus diesem Netz gibt es keine Route dorthin |
| **tmux im Control Mode** | Native Darstellung; Fenster und Panels kannst du anlegen, umbenennen und beenden. Kein winziger Schein-Desktop, der aufs iPhone gequetscht wird |
| **[herdr](https://github.com/herdrdev/herdr)** | Der Multiplexer, der für Coding-Agents gebaut ist, mit jeweils einem Panel in voller Breite |
| **Alle Panels im Blick** | Alle Fenster und Panels der Sitzung auf einmal, jedes als Live-Bild seines Bildschirms, bei tmux wie bei herdr. Zur Seite wischen wechselt zwischen den Panels |
| **Wenn tmux weg ist** | Wird der tmux-Server beendet oder stürzt er ab, zeigt die Übersicht seine Sitzungen und Agents nicht mehr als aktiv, und das Terminal bietet „Neue Sitzung“ an |
| **Mit Metal gezeichnet** | Auf einem Fork des Terminal-Kerns von wezterm: Bilder im Terminal (kitty, iTerm2, sixel), Suchen im Scrollback, Auswahl mit Anfassern und Scrollen durch den Verlauf, das seinen Schwung behält |
| **Eingabe** | Eine Kurzbefehlsleiste für esc / ctrl / alt / Pfeiltasten, IME-Eingabe, antippbare Links, eigene Farbschemata, sieben Terminal-Schriften |
| **Hosts** | Jeder Host bekommt ein Gesicht, das aus seinem Host-Schlüssel gezeichnet wird (derselbe Schlüssel ergibt immer dasselbe Gesicht), oder ein Emoji deiner Wahl. Auf einem neuen iPhone, das aus einem Backup wiederhergestellt wurde, listet Moshpit die Passwörter und Schlüssel auf, die auf dem alten geblieben sind (Einträge im Schlüsselbund ziehen nie mit um), damit du sie neu eingeben oder importieren kannst. Host-Schlüssel, denen du vertraut hast, bleiben vertrauenswürdig |

### Für die Arbeit mit Agents gebaut

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/de-DE/04-island.png" width="220" alt="Der Fortschritt steht in der Dynamic Island.">
</p>

| Funktion | Was dahintersteckt |
| :-- | :-- |
| **„Wartet auf dich“ zuerst** | Die Übersicht zeigt jeden Agent, der auf dich wartet, über alle Hosts hinweg. Darunter, wer arbeitet und wie lange schon, und wer inaktiv ist |
| **Push-Mitteilungen auf dem Sperrbildschirm** | Ende-zu-Ende-verschlüsselt und zugestellt, auch wenn die App geschlossen ist. Ein Tipp darauf, und du bist in dem Panel, das gefragt hat |
| **Live-Aktivität** | Auf dem Sperrbildschirm und in der Dynamic Island: wer wartet, wer arbeitet und wie lange schon |
| **Ein frischer Worktree** | Starte vom iPhone aus einen Agent in einem neuen git worktree: Repository wählen, Branch benennen und auf Wunsch gleich den ersten Prompt mitgeben |
| **In ein Panel teilen** | Teile ein Bild aus jeder beliebigen App direkt in das Panel eines Agents |

### Dateien, die ein Agent erwähnt

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/de-DE/06-preview.png" width="220" alt="Was dein Agent geändert hat, mit einem Tipp öffnen.">
</p>

Tippe in der Ausgabe eines Agents auf einen Pfad, und die Datei öffnet sich auf dem iPhone: Bilder erscheinen als Vorschau, Videos werden abgespielt, Text liest du als Text. Von dort aus kannst du durch ihren Ordner blättern und die Datei weiterteilen.

### Spracheingabe

Diktiert wird auf dem Gerät. **Qwen3-ASR** (0.6B, einmaliger Download) erkennt gesprochenes Chinesisch mit eingestreuten englischen Fachbegriffen als einen zusammenhängenden Satz und nimmt Dateinamen, Branch und Agent, die gerade im Panel stehen, als Hotwords. **Whisper** und Apples Spracherkennung auf dem Gerät sind ebenfalls dabei. Die Aufnahme verlässt das iPhone nie.

### iPad

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/ipad/de-DE/02-home-terminal.png" width="300" alt="Start und Terminal nebeneinander.">
</p>

Links die Hosts, rechts das Terminal. Mit Hardware-Tastatur zeigt <kbd>⌘K</kbd> alle Panels, <kbd>⌘1</kbd>–<kbd>⌘9</kbd> wählt ein Fenster, und <kbd>⌘W</kbd> führt zurück zur Übersicht.

### Der Begleiter und die App-Symbole

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/de-DE/07-pet.png" width="220" alt="Brüte dir einen Pixel-Begleiter fürs Warten aus.">
</p>

| Funktion | Was dahintersteckt |
| :-- | :-- |
| **Ein Pixel-Begleiter** | Er schlüpft in der Übersicht aus einem Ei, setzt sich auf die Karte des Agents, der dich am dringendsten braucht, übernimmt dessen Status und leistet dir in der Dynamic Island und in der Live-Aktivität Gesellschaft. Ist er eingeschaltet, wird auch das Gesicht jedes Hosts zu einem Pixel-Wesen, gezeichnet aus demselben Schlüssel |
| **22 App-Symbole** | Alle in den Einstellungen: standardmäßig ein Pixel-Ei, für jede der sechs Arten des Begleiters eins, auf dem er über ein Panel lugt (jeweils auch in einer schimmernden Variante), dazu Classic und acht weitere. Ist dein Begleiter geschlüpft, fragt die Übersicht, ob du zum Symbol seiner Art wechseln willst |

Alle Screenshots hier sind dieselben wie im App Store. Der Workflow „Screenshots“ erstellt sie und erneuert sie nach jedem vollständigen Lauf. Alle Sprachen und das iPad: [alle Screenshots](https://blog.cluas.eu.org/moshpit/shots/#de-DE).

## Ehrlich von Grund auf

- Einmalkauf. Kein Abo, kein Konto, keine Funktionsstufen.
- Es werden keine Daten gesammelt: keine Analytics, kein Tracking. Schlüssel bleiben im iOS-Schlüsselbund, und der Datenverkehr geht nur an deine eigenen Server, dazu an ein Push-Relay, das nur Chiffretext sieht.
- Moshpit installiert nie etwas auf deinem Host, ohne zu fragen, legt nie hinter deinem Rücken Sitzungen an und sagt Bescheid, wenn es auf etwas Schwächeres zurückfällt, statt stillschweigend das Transportprotokoll zu wechseln.

## Wie es gebaut ist

Moshpit ist größtenteils Rust. Der Kern liegt in `crates/`: der SSH-Transport (`ssh`, auf Basis von russh), das Mosh-Protokoll (`mosh`), die Sitzungsschicht, die Verbindungen aufbaut, nach Abbrüchen neu verbindet und Sitzungen wieder aufnimmt (`session`), der tmux-Control-Mode (`tmux`) und herdr (`herdr`), der Terminal-Emulator (`term`, unser wezterm-Fork in `vendor/wezterm`), was die App speichert (`store`), die Zustandsmaschine der Agents (`agents`), der Begleiter (`pet`) sowie Host-Einrichtung und Push-Kopplung (`hostcare`, `install`, `seal`). Auch die Oberflächen sind in Rust geschrieben, mit [NSH](https://nsh.dev), und liegen in `ui/`, ihre Modelle in `crates/app`.

Unter iOS bindet eine dünne Swift-Schicht all das über eine einzige statische Bibliothek (`crates/ffi`) ein: die App-Hülle, das Zeichnen mit Metal und die Texteingabe der Terminal-Ansicht, den Schlüsselbund, das Diktieren (Qwen3-ASR auf der onnxruntime von sherpa-onnx, Whisper auf WhisperKit, Apples Spracherkennung) und die Teile, die iOS für sich behält (die Live-Aktivität und die Widgets, den Mitteilungsdienst, der Push-Mitteilungen über `crates/seal-ffi` entschlüsselt, die Share-Erweiterung). Eine Android-App nutzt denselben Kern und dieselben Oberflächen (`crates/android`, `android/`); sie ist in Entwicklung und noch nicht veröffentlicht.

<details>
<summary><b>Aufbau des Repositorys</b></summary>

<br>

| Pfad | Was dort liegt |
| :-- | :-- |
| `crates/` | Der Rust-Kern, ein Crate pro Aufgabe: `ssh`, `mosh`, `session`, `tmux`, `herdr`, `term`, `stream` (die Bytes eines Panels zwischen Kern und Terminal-Ansicht), `store`, `agents`, `pet` (der Begleiter), `seal` (der Umschlag für Push-Mitteilungen), `seal-ffi` (der einzige Aufruf des Mitteilungsdienstes in `seal`), `hostcare`, `install`, `app` (die Modelle der Oberflächen), `runtime`, `copy` (die Regeln für die chinesischen Texte), `ffi` (die statische Bibliothek, die iOS linkt) und `android` (die Rust-Hälfte der Android-App) |
| `ui/` | Die Oberflächen, in Rust mit NSH geschrieben, gemeinsam für iOS und Android |
| `vendor/wezterm/` | Unser Fork des Terminal-Emulators von wezterm, hinter `crates/term`; was er ändert, steht in [docs/WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md) |
| `Moshpit/` | Die iOS-Schicht: die App-Hülle, die Swift-Seite jeder Oberfläche, Zeichnen mit Metal und Texteingabe der Terminal-Ansicht, der Schlüsselbund, das Diktieren (Qwen3-ASR, Whisper, Apples Spracherkennung) |
| `Extensions/` | Die drei App-Erweiterungen: `MoshpitIsland/` (Live-Aktivität und Widget für den Sperrbildschirm, Vibe Island), `MoshpitPush/` (Mitteilungsdienst, der Push-Mitteilungen auf dem Gerät entschlüsselt), `MoshpitShare/` (gibt ein Bild an das Panel eines Agents weiter) |
| `Packages/MoshpitKit/` | Lokales Swift-Paket, das die App und alle drei Erweiterungen linken: Logging, die Daten der Live-Aktivität, der Umschlag für verschlüsselte Push-Mitteilungen und der Speicher für die Kopplung |
| `android/` | Die Android-App (Gradle), in Entwicklung |
| `Tests/` | `MoshpitTests/` Unit-Tests, `MoshpitUITests/` UI-Tests |
| `push-relay/` | Das zustandslose Relay in Go zwischen deinem Host und APNs, mit seinem Deployment-Manifest unter `deploy/` |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md), [PUSH.md](docs/PUSH.md) (das Push-Protokoll von Anfang bis Ende), [WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md), `nsh/` (Designnotizen zum Rust-Kern und zu den NSH-Oberflächen), weitere Designnotizen |
| `scripts/` | `rust/` Build der iOS-Bibliothek und ihre Swift-Bindings, `host/` die Shell-Skripte, die auf deinem Entwicklungs-Host installiert werden, `gen/` Generatoren, `verify/` End-to-End-Prüfungen, `spikes/` Experimente, `wezterm/` Importe von Upstream in den Fork, `release/` Werkzeuge für Archive |

</details>

Standardmäßig spricht die App mit dem Relay des Maintainers. [docs/PUSH.md](docs/PUSH.md) erklärt, was das Relay sehen kann und was nicht, und wie du dein eigenes betreibst.

## Aus dem Quellcode bauen

Du brauchst macOS mit Xcode 27 (die App setzt iOS 18 voraus), [XcodeGen](https://github.com/yonaskolb/XcodeGen) per `brew install xcodegen` und Rust über [rustup](https://rustup.rs). Das Repository legt seine Toolchain in `rust-toolchain.toml` fest, deshalb installiert rustup beim ersten Aufruf die richtige.

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # project.yml ist maßgeblich; das .xcodeproj wird daraus erzeugt
open Moshpit.xcodeproj
```

Wähle das Scheme **Moshpit** und einen Simulator und drücke ⌘R. Die Phase „Build Rust“ des App-Targets führt `scripts/rust/build-ios.sh` aus. Das Skript baut `crates/ffi` für Simulator oder Gerät zu `libmoshpit_ffi.a` und fügt beim ersten Mal das nötige Rust-Target hinzu (`aarch64-apple-ios-sim`, `aarch64-apple-ios`). Debug-Builds verwenden das Cargo-Profil `app-test`, Release-Builds `app`. Der erste Build kompiliert den gesamten Rust-Graphen und dauert eine Weile; danach wird inkrementell gebaut. Auch die Swift-Pakete werden beim ersten Build aufgelöst.

Für einen Build aufs Gerät gehört deine Team ID in `Signing.xcconfig`. Der Kommentar in der Datei und [docs/install-free-account.md](docs/install-free-account.md) erklären das Schritt für Schritt, auch für eine kostenlose Apple-ID.

### Tests

Die Rust-Seite, so wie die CI sie ausführt:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p moshpit-term        # nur ein Crate
```

Die iOS-App, so wie die CI sie auf dem Image mit Xcode 27 ausführt:

```sh
scripts/rust/test-build-ios.sh    # prüft Profile und Links der Rust-Phase, ohne zu bauen
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' CODE_SIGNING_ALLOWED=NO
```

Einige Suites werden in einem unsignierten Build übersprungen (sie brauchen den Container der App Group) oder außerhalb des iPad-Simulators (Tastaturkurzbefehle für Hardware-Tastaturen), und manche UI-Abläufe bleiben ausgelassen, bis sie für die aktuellen Oberflächen neu geschrieben sind. Das Log nennt jeden einzelnen Fall.

Die Android-App wird mit `android/ci.sh` gebaut und getestet. Dafür brauchst du ein JDK 17; den Rest (die Teile des SDK, das NDK, Gradle) holt sich das Skript selbst.

## Mitmachen

Lies [CONTRIBUTING.md](CONTRIBUTING.md) für den Ablauf und die Stilregeln des Projekts und [SECURITY.md](SECURITY.md), bevor du etwas Sicherheitsrelevantes meldest. Für alle hier gilt der [Verhaltenskodex](CODE_OF_CONDUCT.md).

Moshpit ist auch auf [LINUX DO](https://linux.do) zu finden, einer chinesischsprachigen Community für Entwickler. Fragen und Ideen sind dort genauso willkommen wie im Issue-Tracker.

## Lizenz

Moshpit ist freie Software, veröffentlicht unter der [GNU General Public License v3.0](LICENSE) (GPL-3.0-only). Du darfst es unter den Bedingungen dieser Lizenz bauen, verändern und weitergeben. Den Build im App Store veröffentlicht der Maintainer, der das Urheberrecht hält. Komponenten von Drittanbietern behalten ihre eigenen Lizenzen; sie sind in [NOTICES.md](NOTICES.md) aufgeführt.
