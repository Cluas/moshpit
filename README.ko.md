<!-- source: README.md 6c585bea06aec83880bcb57346ee39d0aa3a9b2b -->
<div align="center">

<img src="Moshpit/Resources/AlternateIcons/Previews/IconPreview-Moshpit@3x.png" width="112" alt="Moshpit 아이콘: 터미널 패널 너머로 고개를 내민 픽셀 알">

# Moshpit

### 내 머신에서 돌아가는 코딩 에이전트를 위한 터미널.

**Claude Code, Codex, Gemini는 서버에서 계속 일하고, 내 답이 필요해지면 휴대폰이 먼저 압니다.**<br>
iPhone과 iPad에서 쓰는 SSH, Mosh, tmux, herdr 터미널입니다. 질문마다 암호화된 푸시 알림이 오고,<br>
실시간 현황에는 누가 기다리는지, 누가 작업 중인지, 각각 얼마나 됐는지 표시됩니다.

[![App Store](https://img.shields.io/badge/App_Store-iPhone_%26_iPad-111111?logo=apple&logoColor=white)](https://apps.apple.com/app/id6799896801) ![iOS and iPadOS 18+](https://img.shields.io/badge/iOS_%7C_iPadOS-18%2B-111111?logo=apple&logoColor=white) [![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-111111)](LICENSE)

**[App Store](https://apps.apple.com/app/id6799896801)** &nbsp;·&nbsp; **[웹사이트(영문)](https://moshpit.cluas.eu.org)** &nbsp;·&nbsp; **[문서(영문)](https://moshpit.cluas.eu.org/docs)** &nbsp;·&nbsp; **[이슈](https://github.com/Cluas/moshpit/issues)**

[English](README.md) · [简体中文](README-zh.md) · [繁體中文](README.zh-Hant.md) · [日本語](README.ja.md) · **한국어** · [Deutsch](README.de.md) · [Español](README.es.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md)

<br>

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ko/01-lock.png" width="250" alt="누가 무엇을 기다리는지 잠금 화면에서 바로 보여요"> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ko/03-home.png" width="250" alt="누가 일하고 누가 기다리는지 한눈에"> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ko/02-pane.png" width="250" alt="탭하면 물어본 그 패널로 바로">

</div>

<br>

## 누구를 위한 앱인가

SSH로 접속하는 머신(워크스테이션, 서버, 클라우드 인스턴스)에서 Claude Code, Codex, Gemini CLI 같은 코딩 에이전트를 돌리고, 그 에이전트가 대개 tmux나 herdr 안에 있으며, 질문하는 에이전트에게 노트북 앞으로 돌아가지 않고 휴대폰으로 답하고 싶다면 Moshpit이 맞습니다.

- **내가 필요할 때 알 수 있습니다.** 에이전트의 질문은 푸시 알림으로 잠금 화면에 도착합니다. 호스트에서 암호화되고 휴대폰에서 풀리며, 한 번 탭하면 질문한 그 패널로 들어갑니다.
- **모든 에이전트를 한눈에 봅니다.** 홈은 모든 호스트에서 나를 기다리는 에이전트를 먼저 보여 주고, 그다음 작업 중인 에이전트와 작업 시간, 마지막으로 대기 중인 에이전트를 보여 줍니다.
- **제대로 된 터미널입니다.** 네트워크가 바뀌어도 끊기지 않는 Mosh, 네이티브로 다루는 tmux 창과 패널, 단축키 막대, 인라인 이미지를 갖췄습니다.

에이전트가 내가 접속할 수 있는 머신에서 돌지 않는다면 맞지 않습니다. Moshpit은 터미널이지, 에이전트를 실행하는 곳도 아니고 호스팅된 에이전트와 대화하는 채팅 앱도 아닙니다. iPhone과 iPad 전용이며, Android 앱은 개발 중이고 아직 출시되지 않았습니다.

## 빠른 시작

**1 · 설치.** [App Store](https://apps.apple.com/app/id6799896801)에서 받거나(한 번만 결제하는 유료 앱), [소스에서 직접 빌드](#소스에서-빌드)하세요.

**2 · 호스트 추가.** 주소와 사용자 이름을 입력하고, 로그인 방법으로 암호나 SSH 키를 고르세요. 키는 휴대폰에서 생성하거나, 가져오거나, PEM을 그대로 붙여넣으면 됩니다. SSH 서버라면 어디든 연결됩니다.

**3 · 호스트 설정(선택).** *호스트 설정*은 에이전트마다 상태를 알리는 훅을 설치하고, 푸시 알림을 받을 수 있도록 호스트를 페어링합니다. 호스트에서 무엇이든 바꾸기 전에 먼저 묻습니다. Codex는 호스트에서 한 단계가 더 필요합니다. Codex에서 `/hooks`를 실행하고 Moshpit 항목 네 개를 신뢰하세요.

**4 · Mosh, tmux, herdr 추가(선택).** Mosh는 네트워크가 바뀌어도 세션을 유지하고, tmux와 herdr는 자기 패널을 네이티브로 보여 줍니다. 앱 안에 각각의 설치 안내가 있습니다.

## 지원하는 에이전트

| 에이전트 | 홈, Dynamic Island, 실시간 현황의 상태 | “기다리는 중” 푸시 알림 | “완료” 푸시 알림 |
| :-- | :-: | :-: | :-: |
| Claude Code | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ (권한 요청) | ✓ |
| Gemini CLI | ✓ | ✓ (도구 권한) | ✓ |
| Qwen Code | ✓ | ✓ | ✓ |
| Qoder | ✓ | ✓ | ✓ |
| Factory (`droid`) | ✓ | ✓ | ✓ |
| CodeBuddy | ✓ | ✓ | ✓ |

*호스트 설정*은 호스트에서 찾은 에이전트에만 훅을 설치합니다. tmux에서는 홈에 보이는 상태가 이 훅에서 오고, herdr에서는 herdr가 직접 알려 주므로 따로 설정할 것이 없습니다. 푸시 알림은 에이전트가 어디서 돌든 훅이 보냅니다. tmux 안이든, herdr 안이든, 일반 셸이든 마찬가지입니다.

“기다리는 중” 푸시 알림은 먼저 30초를 기다립니다. 그래서 책상 앞에서 바로 답한 질문은 휴대폰까지 오지 않습니다. “완료” 푸시 알림에는 이번 턴이 얼마나 걸렸는지가 담기고, 호스트에 `jq`가 있으면 어떤 프롬프트에 답했는지도 담깁니다. 3분 이상 걸린 턴은 소리로 알리고, 그보다 짧은 턴은 조용히 도착합니다.

## 동작 방식

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/architecture-dark.svg">
  <img src="docs/assets/architecture-light.svg" width="900" alt="Moshpit의 구조: iPhone과 iPad의 앱(Rust 코어 위에 얇은 Swift 셸)이 SSH나 Mosh로 호스트에 연결해 tmux나 herdr 패널과 에이전트마다의 상태를 읽습니다. 호스트에서는 에이전트의 훅이 패널마다 상태를 기록하고, ‘기다리는 중’과 ‘완료’를 암호화 스크립트에 넘깁니다. 암호화된 푸시는 암호문만 보는 릴레이와 APNs를 거쳐 휴대폰의 알림 서비스에 도착하고, 거기서 풀립니다.">
</picture>

앱은 SSH나 Mosh로 호스트에 연결하고, tmux는 컨트롤 모드로, herdr는 herdr 자체 프로토콜로 다룹니다. 그래서 패널은 네이티브로 그려지고, 에이전트 상태는 출력에서 추측하는 것이 아니라 데이터로 전달됩니다. 호스트에서는 에이전트의 훅이 작은 스크립트를 호출합니다. 이 스크립트는 패널에 에이전트 상태를 기록하고, “기다리는 중”과 “완료”일 때는 *호스트 설정*에서 페어링한 키로 푸시를 암호화합니다. 릴레이는 암호화된 봉투를 APNs로 넘길 뿐 암호문밖에 보지 못하고, 휴대폰의 알림 서비스가 이를 풉니다. 프로토콜의 처음부터 끝까지는 [docs/PUSH.md](docs/PUSH.md)에 있습니다.

## 기능

### 먼저, 제대로 된 터미널

<table>
<tr>
<td width="50%"><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ko/05-overview.png" alt="tmux, herdr 패널을 한 화면에"></td>
<td width="50%"><img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ko/08-keyboard.png" alt="터미널을 위해 만든 키보드"></td>
</tr>
</table>

| | |
| :-- | :-- |
| **SSH와 Mosh** | Mosh는 예측 에코를 지원합니다. Wi-Fi와 5G 사이 전환, 잠자기, 불안정한 호텔 네트워크에서도 세션이 유지되고, 앱이 백그라운드에 있는 동안 살아남은 연결은 다시 연결하지 않고 그대로 이어서 씁니다 |
| **연결이 안 되는 이유를 알려 줍니다** | 포트가 연결을 거부했는지, 이름을 찾지 못했는지(또는 로컬 네트워크 권한이 꺼져 있는지), 호스트가 응답하지 않았는지(VPN을 확인하세요), 이 네트워크에서 가는 경로가 없는지 구분해서 보여 줍니다 |
| **tmux 컨트롤 모드** | 네이티브 렌더링이며, 창과 패널을 만들고 이름을 변경하고 종료할 수 있습니다. 휴대폰에 억지로 밀어 넣은 작은 가짜 데스크톱이 아닙니다 |
| **[herdr](https://github.com/herdrdev/herdr)** | 코딩 에이전트를 위해 만든 멀티플렉서. 한 번에 패널 하나를 화면 너비 가득 보여 줍니다 |
| **패널 개요** | 세션의 모든 창과 패널을 한 화면에 펼치고, 각각을 실시간 화면으로 보여 줍니다. tmux와 herdr 모두 같습니다. 옆으로 밀면 패널 사이를 오갑니다 |
| **tmux가 사라지면** | tmux 서버가 종료되거나 충돌하면 홈은 그 세션과 에이전트를 더 이상 살아 있는 것으로 표시하지 않고, 터미널은 “새 세션”을 제안합니다 |
| **Metal로 그리는 화면** | wezterm 터미널 코어를 포크해 씁니다. 인라인 이미지(kitty, iTerm2, sixel), 스크롤백 찾기, 핸들로 조절하는 선택 영역, 관성이 살아 있는 히스토리 스크롤을 지원합니다 |
| **입력** | esc / ctrl / alt / 방향키를 담은 단축키 막대, 입력기 조합 입력, 탭하면 열리는 링크, 사용자 지정 테마, 터미널 서체 일곱 가지 |
| **호스트** | 호스트마다 호스트 키에서 그려 낸 얼굴(같은 키는 언제나 같은 얼굴)이나 직접 고른 이모지가 붙습니다. 백업에서 복원한 새 iPhone에서는 이전 iPhone에 남은 암호와 키를 목록으로 보여 주므로(키체인 항목은 옮겨지지 않습니다) 다시 입력하거나 가져올 수 있고, 신뢰했던 호스트 키는 계속 신뢰합니다 |

### 에이전트 작업을 위해

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ko/04-island.png" width="250" align="right" alt="앱을 안 열어도 진행 상황이 보여요">

| | |
| :-- | :-- |
| **“기다리는 중”이 맨 위에** | 홈은 모든 호스트에서 나를 기다리는 에이전트를 먼저 보여 줍니다. 그 아래로 작업 중인 에이전트와 작업 시간, 대기 중인 에이전트가 이어집니다 |
| **잠금 화면의 푸시 알림** | 종단 간 암호화되며, 앱이 닫혀 있어도 도착합니다. 탭하면 질문한 그 패널로 들어갑니다 |
| **실시간 현황** | 잠금 화면과 Dynamic Island에 누가 기다리는지, 누가 작업 중인지, 각각 얼마나 됐는지 표시됩니다 |
| **새 워크트리** | 휴대폰에서 새 git worktree를 만들고 에이전트를 시작합니다. 저장소를 고르고, 브랜치 이름을 정하고, 원하면 첫 프롬프트도 함께 넘깁니다 |
| **패널로 공유** | 어떤 앱에서든 이미지를 에이전트의 패널로 바로 공유합니다 |

<br clear="right">

### 에이전트가 언급한 파일

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ko/06-preview.png" width="250" align="right" alt="에이전트가 고친 파일, 탭 한 번으로 열기">

에이전트 출력에 나온 경로를 탭하면 그 파일이 휴대폰에서 열립니다. 이미지는 미리 보고, 동영상은 재생하고, 텍스트는 텍스트로 읽습니다. 거기서 파일이 있는 폴더를 둘러보거나 파일을 다른 곳으로 공유할 수 있습니다.

<br clear="right">

### 음성 입력

받아쓰기는 기기 안에서 처리됩니다. **Qwen3-ASR**(0.6B, 처음 한 번만 내려받음)은 영어 용어가 섞인 중국어를 한 문장으로 알아듣고, 패널 화면에 보이는 파일 이름, 브랜치, 에이전트를 핫워드로 씁니다. **Whisper**와 Apple의 온디바이스 엔진도 쓸 수 있습니다. 음성은 휴대폰 밖으로 나가지 않습니다.

### iPad

<img src="https://blog.cluas.eu.org/moshpit/shots/ipad/ko/02-home-terminal.png" width="360" align="right" alt="홈과 터미널을 나란히">

왼쪽에는 호스트, 오른쪽에는 터미널이 있습니다. 하드웨어 키보드에서는 <kbd>⌘K</kbd>로 모든 패널을 보고, <kbd>⌘1</kbd>–<kbd>⌘9</kbd>로 창을 고르고, <kbd>⌘W</kbd>로 홈에 돌아갑니다.

<br clear="right">

### 컴패니언과 아이콘

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/ko/07-pet.png" width="250" align="right" alt="같이 기다려 줄 픽셀 친구를 키워요">

| | |
| :-- | :-- |
| **픽셀 컴패니언** | 홈에서 알을 깨고 나와, 나를 가장 급하게 기다리는 에이전트의 카드 위에 올라앉아 그 에이전트의 상태를 따라 하고, Dynamic Island와 실시간 현황에서도 곁을 지킵니다. 켜 두면 호스트마다의 얼굴도 같은 키에서 그려 낸 픽셀 크리처로 바뀝니다 |
| **아이콘 22개** | 모두 설정에 있습니다. 기본은 픽셀 알이고, 컴패니언 여섯 종이 각각 패널 너머로 고개를 내민 아이콘(종마다 반짝이 하나씩), Classic, 그리고 여덟 개가 더 있습니다. 컴패니언이 부화하면 홈에서 아이콘을 그 종으로 바꿀지 묻습니다 |

<br clear="right">

여기 실린 스크린샷은 모두 App Store용 세트로, Screenshots 워크플로가 만들고 전체 실행이 끝날 때마다 새로 바뀝니다. 모든 언어와 iPad 스크린샷은 [전체 스크린샷](https://blog.cluas.eu.org/moshpit/shots/#ko)에서 볼 수 있습니다.

## 숨기는 것 없이

- 한 번 결제하면 끝입니다. 구독도, 계정도, 기능 등급도 없습니다.
- 아무것도 수집하지 않습니다. 분석도 추적도 없습니다. 키는 iOS 키체인에 남고, 트래픽은 내 서버와 암호문만 보는 푸시 릴레이로만 갑니다.
- Moshpit은 묻지 않고 호스트에 아무것도 설치하지 않고, 몰래 세션을 만들지 않으며, 연결 상태가 나빠지면 조용히 전송 방식을 바꾸지 않고 그렇다고 알려 줍니다.

## 내부 구현

Moshpit은 대부분 Rust로 작성되어 있습니다. 코어는 `crates/`에 있습니다. SSH 전송(`ssh`, russh 기반), Mosh 프로토콜(`mosh`), 연결·재연결·재개를 맡는 세션 계층(`session`), tmux 컨트롤 모드(`tmux`)와 herdr(`herdr`), 터미널 에뮬레이터(`term`, `vendor/wezterm`에 있는 wezterm 포크), 앱이 저장하는 데이터(`store`), 에이전트 상태 머신(`agents`), 컴패니언(`pet`), 호스트 설정과 푸시 페어링(`hostcare`, `install`, `seal`)이 여기에 들어 있습니다. 화면도 [NSH](https://nsh.dev)를 써서 Rust로 작성해 `ui/`에 두었고, 화면의 모델은 `crates/app`에 있습니다.

iOS에서는 얇은 Swift 계층이 정적 라이브러리 하나(`crates/ffi`)를 통해 이 모든 것을 올립니다. Swift가 맡는 것은 앱 셸, 터미널 뷰의 Metal 렌더링과 텍스트 입력, 키체인, 받아쓰기(sherpa-onnx의 onnxruntime 위에서 도는 Qwen3-ASR, WhisperKit 위의 Whisper, Apple 음성 엔진), 그리고 iOS가 직접 맡는 부분(실시간 현황과 위젯, `crates/seal-ffi`로 푸시를 푸는 알림 서비스, 공유 확장)입니다. Android 앱도 같은 코어와 화면을 공유하며(`crates/android`, `android/`), 개발 중이고 아직 출시되지 않았습니다.

<details>
<summary><b>저장소 구조</b></summary>

<br>

| 경로 | 들어 있는 것 |
| :-- | :-- |
| `crates/` | Rust 코어. 관심사마다 크레이트 하나씩: `ssh`, `mosh`, `session`, `tmux`, `herdr`, `term`, `stream`(코어와 터미널 뷰 사이를 오가는 패널의 바이트), `store`, `agents`, `pet`(컴패니언), `seal`(푸시 봉투), `seal-ffi`(알림 서비스가 `seal`을 부르는 단 하나의 호출), `hostcare`, `install`, `app`(화면의 모델), `runtime`, `copy`(중국어 문구가 지키는 규칙), `ffi`(iOS가 링크하는 정적 라이브러리), `android`(Android 앱의 Rust 쪽 절반) |
| `ui/` | NSH로 Rust로 작성한 화면. iOS와 Android가 함께 씁니다 |
| `vendor/wezterm/` | `crates/term` 뒤에 있는 wezterm 터미널 에뮬레이터 포크. 무엇을 바꿨는지는 [docs/WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md)에 있습니다 |
| `Moshpit/` | iOS 호스트: 앱 셸, 각 화면의 Swift 쪽, 터미널 뷰의 Metal 렌더링과 텍스트 입력, 키체인, 받아쓰기(Qwen3-ASR, Whisper, Apple 엔진) |
| `Extensions/` | 앱 확장 세 개: `MoshpitIsland/`(실시간 현황과 잠금 화면 위젯, Vibe Island), `MoshpitPush/`(기기에서 푸시를 복호화하는 알림 서비스), `MoshpitShare/`(이미지를 에이전트의 패널로 넘김) |
| `Packages/MoshpitKit/` | 앱과 확장 세 개가 모두 링크하는 로컬 Swift 패키지: 로깅, 실시간 현황 페이로드, 암호화된 푸시 봉투와 페어링 저장소 |
| `android/` | Android 앱(Gradle), 개발 중 |
| `Tests/` | `MoshpitTests/` 단위 테스트, `MoshpitUITests/` UI 테스트 |
| `push-relay/` | 호스트와 APNs 사이에 있는 상태 없는 Go 릴레이. 배포 매니페스트는 `deploy/`에 있습니다 |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md), [PUSH.md](docs/PUSH.md)(푸시 프로토콜 전체), [WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md), `nsh/`(Rust 코어와 NSH 화면의 설계 노트), 그 밖의 설계 노트 |
| `scripts/` | `rust/` iOS 라이브러리 빌드와 Swift 바인딩, `host/` 개발 호스트에 설치되는 셸 스크립트, `gen/` 생성기, `verify/` 엔드투엔드 검사, `spikes/` 실험, `wezterm/` 포크의 업스트림 가져오기, `release/` 아카이브 도구 |

</details>

앱은 기본적으로 메인테이너가 운영하는 릴레이를 씁니다. 릴레이가 무엇을 볼 수 있고 무엇을 볼 수 없는지, 직접 운영하려면 어떻게 하는지는 [docs/PUSH.md](docs/PUSH.md)에 있습니다.

## 소스에서 빌드

Xcode 27이 설치된 macOS(앱의 배포 대상은 iOS 18), `brew install xcodegen`으로 설치한 [XcodeGen](https://github.com/yonaskolb/XcodeGen), [rustup](https://rustup.rs)으로 설치한 Rust가 필요합니다. 저장소가 `rust-toolchain.toml`에 툴체인을 고정해 두었으므로, rustup이 처음 쓸 때 맞는 버전을 설치합니다.

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # project.yml이 원본이고, .xcodeproj는 여기서 생성됩니다
open Moshpit.xcodeproj
```

**Moshpit** 스킴과 시뮬레이터를 고르고 ⌘R을 누르세요. 앱 타깃의 “Build Rust” 단계가 `scripts/rust/build-ios.sh`를 실행해 `crates/ffi`를 시뮬레이터용 또는 기기용 `libmoshpit_ffi.a`로 빌드하며, 처음에는 필요한 Rust 타깃(`aarch64-apple-ios-sim`, `aarch64-apple-ios`)도 추가합니다. Debug 빌드는 cargo의 `app-test` 프로필을, Release 빌드는 `app` 프로필을 씁니다. 첫 빌드는 Rust 전체를 컴파일하므로 시간이 걸리고, 이후 빌드는 증분 빌드입니다. Swift 패키지도 첫 빌드 때 받아 옵니다.

기기용 빌드에는 `Signing.xcconfig`에 내 Team ID를 넣어야 합니다. 그 파일의 주석과 [docs/install-free-account.md](docs/install-free-account.md)에 무료 Apple ID로 하는 방법까지 차근차근 나와 있습니다.

### 테스트

Rust 쪽은 CI가 돌리는 그대로입니다.

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p moshpit-term        # 크레이트 하나만
```

iOS 앱은 CI가 Xcode 27 이미지에서 돌리는 그대로입니다.

```sh
scripts/rust/test-build-ios.sh    # 빌드하지 않고 Rust 단계의 프로필과 링크만 확인
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' CODE_SIGNING_ALLOWED=NO
```

일부 테스트 묶음은 서명하지 않은 빌드에서(App Group 컨테이너가 필요합니다) 또는 iPad 시뮬레이터가 아닌 곳에서(하드웨어 키보드 단축키) 건너뛰고, 일부 UI 흐름은 지금 화면에 맞게 다시 작성될 때까지 건너뜁니다. 건너뛴 항목은 로그에 하나하나 이름이 남습니다.

Android 앱은 `android/ci.sh`로 빌드하고 테스트합니다. JDK 17만 있으면 필요한 것(SDK 구성 요소, NDK, Gradle)은 이 스크립트가 알아서 받아 옵니다.

## 기여하기

작업 흐름과 스타일 규칙은 [CONTRIBUTING.md](CONTRIBUTING.md)(영문)에 있고, 민감한 문제를 제보하기 전에는 [SECURITY.md](SECURITY.md)를 먼저 읽어 주세요. 이 프로젝트의 모든 참여자는 [행동 강령](CODE_OF_CONDUCT.md)을 따릅니다.

Moshpit은 중국어 개발자 커뮤니티인 [LINUX DO](https://linux.do)에도 소개되어 있습니다. 질문이나 아이디어는 이슈 트래커와 마찬가지로 그곳에서도 환영합니다.

## 라이선스

Moshpit은 자유 소프트웨어이며 [GNU 일반 공중 사용 허가서 v3.0](LICENSE)(GPL-3.0-only)으로 배포됩니다. 이 라이선스의 조건에 따라 빌드하고, 수정하고, 재배포할 수 있습니다. App Store 빌드는 저작권을 가진 메인테이너가 배포합니다. 서드파티 구성 요소는 각자의 라이선스를 따르며, 목록은 [NOTICES.md](NOTICES.md)에 있습니다.
