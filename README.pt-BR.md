<!-- source: README.md 2d4fb4a3e4f6551abe6e997954516d14aadde7dd -->
<div align="center">

<img src="Moshpit/Resources/AlternateIcons/Previews/IconPreview-Moshpit@3x.png" width="112" alt="Ícone do Moshpit: um ovo em pixels espiando por cima de um painel de terminal">

# Moshpit

### O terminal dos agentes de programação que rodam nas suas próprias máquinas.

**Claude Code, Codex e Gemini seguem trabalhando no servidor. Quando um deles precisa de você, o seu celular fica sabendo.**<br>
SSH, Mosh, tmux e herdr no iPhone e no iPad, com uma notificação push criptografada para cada pergunta<br>
e uma Atividade ao Vivo mostrando quem está esperando, quem está trabalhando e há quanto tempo.

[![App Store](https://img.shields.io/badge/App_Store-iPhone_%26_iPad-111111?logo=apple&logoColor=white)](https://apps.apple.com/app/id6799896801) ![iOS and iPadOS 18+](https://img.shields.io/badge/iOS_%7C_iPadOS-18%2B-111111?logo=apple&logoColor=white) [![License: GPL-3.0-only](https://img.shields.io/badge/license-GPL--3.0--only-111111)](LICENSE)

**[App Store](https://apps.apple.com/app/id6799896801)** &nbsp;·&nbsp; **[Site (em inglês)](https://moshpit.cluas.eu.org)** &nbsp;·&nbsp; **[Documentação (em inglês)](https://moshpit.cluas.eu.org/docs)** &nbsp;·&nbsp; **[Issues](https://github.com/Cluas/moshpit/issues)**

[English](README.md) · [简体中文](README-zh.md) · [繁體中文](README.zh-Hant.md) · [日本語](README.ja.md) · [한국어](README.ko.md) · [Deutsch](README.de.md) · [Español](README.es.md) · [Français](README.fr.md) · **Português (Brasil)**

<br>

<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/pt-BR/01-lock.png" width="250" alt="Quem precisa de você e para quê, na tela bloqueada."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/pt-BR/03-home.png" width="250" alt="Quem está trabalhando e quem está esperando, num relance."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/pt-BR/02-pane.png" width="250" alt="Toque e caia direto no painel que perguntou.">

</div>

<br>

## O Moshpit é para você?

É, se você roda Claude Code, Codex, Gemini CLI ou outro agente de programação numa máquina que acessa por SSH (uma estação de trabalho, um servidor, uma instância na nuvem), muitas vezes dentro do tmux ou do herdr, e quer responder pelo celular ao agente que está perguntando, em vez de voltar até o notebook:

- **Saiba quando precisam de você.** A pergunta chega à Tela Bloqueada como notificação push, criptografada no seu host e aberta no celular, e um toque leva você direto ao painel que perguntou.
- **Veja todos os agentes de uma vez.** A tela Início mostra primeiro quem está esperando você, em todos os seus hosts; depois, quem está trabalhando e há quanto tempo; por fim, quem está ocioso.
- **Use um terminal de verdade.** Mosh que aguenta troca de rede, janelas e painéis do tmux gerenciados de forma nativa, barra de atalhos, imagens dentro do terminal.

Não é, se os seus agentes não rodam numa máquina que você consegue acessar: o Moshpit é um terminal, não um lugar para rodar agentes nem um app de chat para um agente hospedado. Ele existe só para iPhone e iPad; o app para Android está em desenvolvimento e não foi lançado.

## Primeiros passos

**1 · Instale** o Moshpit pela [App Store](https://apps.apple.com/app/id6799896801) (compra única) ou [compile você mesmo](#compilar-do-código-fonte).

**2 · Adicione um host.** Informe o endereço e o usuário e entre com senha ou chave SSH: gere uma chave no celular, importe uma ou cole um PEM. Funciona com qualquer servidor SSH.

**3 · Configure o host (opcional).** *Configurar um host* instala os hooks que informam o status de cada agente e pareia o host para receber notificações push. Antes de mudar qualquer coisa no host, ele pergunta. O Codex pede mais um passo lá: rode `/hooks` no Codex e confie nas quatro entradas do Moshpit.

**4 · Adicione Mosh, tmux ou herdr (opcional).** O Mosh mantém a sessão quando a rede muda; tmux e herdr mostram os painéis deles de forma nativa. O app traz um guia de instalação para cada um.

## Agentes compatíveis

| Agente | Status | Esperando você | Concluído |
| :-- | :-: | :-: | :-: |
| Claude Code | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ ¹ | ✓ |
| Gemini CLI | ✓ | ✓ ² | ✓ |
| Qwen Code | ✓ | ✓ | ✓ |
| Qoder | ✓ | ✓ | ✓ |
| Factory (`droid`) | ✓ | ✓ | ✓ |
| CodeBuddy | ✓ | ✓ | ✓ |

O **status** aparece no Início, na Dynamic Island e na Atividade ao Vivo; **Esperando você** e **Concluído** são notificações push. ¹ O Codex avisa pelas solicitações de permissão. ² O Gemini CLI só avisa quando uma ferramenta pede permissão.

*Configurar um host* instala os hooks dos agentes que encontra no host. No tmux, o status que aparece no Início vem desses hooks; no herdr, quem informa é o próprio herdr, sem configuração nenhuma. As notificações push saem dos hooks onde quer que o agente rode: no tmux, no herdr ou num shell comum.

A notificação “Esperando você” aguarda 30 segundos antes de sair, então uma pergunta que você responde na mesa nunca chega ao celular. A notificação “Concluído” diz quanto tempo o turno levou e, se o host tiver `jq`, a qual prompt ele respondeu; um turno de três minutos ou mais toca um som, um mais curto chega em silêncio.

## Como as peças se encaixam

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/architecture-dark.svg">
  <img src="docs/assets/architecture-light.svg" width="900" alt="Como o Moshpit se encaixa: o app no iPhone e no iPad (uma camada fina de Swift sobre um núcleo em Rust) acessa o seu host por SSH ou Mosh e lê os painéis do tmux ou do herdr e o status de cada agente. No host, os hooks dos agentes marcam cada painel com o status dele e entregam “Esperando você” e “Concluído” a um script que os criptografa; a notificação criptografada passa por um relay que só vê texto cifrado e pelo APNs até chegar ao serviço de notificação do celular, que a abre.">
</picture>

O app acessa o seu host por SSH ou Mosh e controla o tmux pelo modo de controle dele e o herdr pelo protocolo do próprio herdr. Por isso os painéis são desenhados de forma nativa, e o status de cada agente chega como dado, em vez de ser adivinhado pela saída. No host, os hooks de cada agente chamam um pequeno script que marca o painel com o status do agente e, no caso de “Esperando você” e “Concluído”, criptografa uma notificação com a chave pareada em *Configurar um host*. O relay repassa esse envelope criptografado ao APNs e só enxerga texto cifrado; quem o abre é o serviço de notificação no celular. O protocolo inteiro, de ponta a ponta, está em [docs/PUSH.md](docs/PUSH.md).

## O que ele faz

### Antes de tudo, um terminal

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/pt-BR/05-overview.png" width="220" alt="Todos os painéis do tmux e do herdr numa tela só."> <img src="https://blog.cluas.eu.org/moshpit/shots/iphone/pt-BR/08-keyboard.png" width="220" alt="Um teclado feito para o terminal.">
</p>

| Recurso | O que significa |
| :-- | :-- |
| **SSH e Mosh** | Mosh com eco preditivo. As sessões sobrevivem à troca entre Wi-Fi e 5G, ao repouso do aparelho e à rede ruim de hotel, e uma conexão que continuou viva enquanto o app estava em segundo plano é retomada sem reconectar |
| **Diz por que não conecta** | A porta recusou a conexão, o nome não resolveu (ou a permissão de Rede Local está desligada), o host não respondeu (confira sua VPN) ou não há rota a partir desta rede |
| **Modo de controle do tmux** | Renderização nativa, com janelas e painéis que você cria, renomeia e mata. Nada de uma área de trabalho minúscula espremida na tela do celular |
| **[herdr](https://github.com/herdrdev/herdr)** | O multiplexador feito para agentes de programação, um painel de largura total por vez |
| **Visão geral dos painéis** | Todas as janelas e painéis da sessão de uma vez, cada um com a imagem ao vivo da sua tela, tanto no tmux quanto no herdr. Deslize para os lados para passar de um painel a outro |
| **Quando o tmux some** | Se o servidor do tmux for morto ou cair, o Início deixa de mostrar as sessões e os agentes dele como ativos, e o terminal oferece “Nova sessão” |
| **Desenhado com Metal** | Sobre um fork do núcleo de terminal do wezterm: imagens inline (kitty, iTerm2, sixel), busca no histórico de rolagem, seleção com alças e rolagem pelo histórico que mantém a inércia |
| **Digitação** | Barra de atalhos com esc / ctrl / alt / setas, composição de IME, links tocáveis, temas personalizados, sete fontes de terminal |
| **Hosts** | Cada host ganha um rosto desenhado a partir da chave do host (a mesma chave, sempre o mesmo rosto) ou um emoji que você escolhe. Num iPhone novo restaurado de um backup, o Moshpit lista as senhas e chaves que ficaram no aparelho antigo (itens das Chaves nunca vão junto) para você digitar ou importar de novo, e as chaves de host em que você já confiava continuam confiáveis |

### Feito para trabalhar com agentes

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/pt-BR/04-island.png" width="220" alt="O progresso aparece na Dynamic Island.">
</p>

| Recurso | O que significa |
| :-- | :-- |
| **“Esperando você” em primeiro lugar** | O Início lista todos os agentes que estão esperando você, em todos os seus hosts. Logo abaixo, quem está trabalhando, há quanto tempo, e quem está ocioso |
| **Notificações na Tela Bloqueada** | Criptografadas de ponta a ponta e entregues mesmo com o app fechado. Toque numa e você cai no painel que perguntou |
| **Atividade ao Vivo** | Na Tela Bloqueada e na Dynamic Island: quem está esperando, quem está trabalhando e há quanto tempo |
| **Um worktree novo** | Inicie um agente num git worktree novo pelo celular: escolha um repositório, dê nome a um branch e, se quiser, passe o primeiro prompt |
| **Compartilhar num painel** | Compartilhe uma imagem de qualquer app direto no painel de um agente |

### Arquivos que o agente menciona

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/pt-BR/06-preview.png" width="220" alt="Abra na hora o arquivo que o agente acabou de alterar.">
</p>

Toque num caminho na saída de um agente e o arquivo abre no celular: imagem aparece em prévia, vídeo toca, texto abre como texto. A partir dali, dá para navegar pela pasta dele e compartilhar o arquivo adiante.

### Voz

O ditado roda no aparelho. O **Qwen3-ASR** (0.6B, baixado uma única vez) entende fala em chinês cheia de termos em inglês como uma frase só e usa como hotwords os nomes de arquivo, o branch e o agente que aparecem na tela do painel. O **Whisper** e os mecanismos de reconhecimento da própria Apple, que rodam no aparelho, também estão lá. O áudio nunca sai do celular.

### iPad

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/ipad/pt-BR/02-home-terminal.png" width="300" alt="Início e terminal, lado a lado.">
</p>

Hosts à esquerda, terminal à direita. Com teclado físico, <kbd>⌘K</kbd> mostra todos os painéis, <kbd>⌘1</kbd>–<kbd>⌘9</kbd> escolhe uma janela e <kbd>⌘W</kbd> volta para o Início.

### O companheiro e os ícones

<p align="center">
<img src="https://blog.cluas.eu.org/moshpit/shots/iphone/pt-BR/07-pet.png" width="220" alt="Choque um amiguinho em pixel para esperar com você.">
</p>

| Recurso | O que significa |
| :-- | :-- |
| **Um companheiro em pixels** | Ele sai de um ovo no Início, pousa no cartão do agente que mais precisa de você, assume o status desse agente e faz companhia na Dynamic Island e na Atividade ao Vivo. Com ele ligado, o rosto de cada host vira uma criatura em pixels, desenhada a partir da mesma chave |
| **Vinte e dois ícones** | Todos em Ajustes: um ovo em pixels por padrão, o companheiro espiando por cima de um painel para cada uma das suas seis espécies (cada uma com uma versão brilhante), o Classic e mais oito. Quando o seu companheiro nasce, o Início pergunta se você quer trocar para o ícone da espécie dele |

Todas as capturas de tela daqui são do conjunto da App Store, geradas pelo workflow Screenshots e atualizadas a cada execução completa. Todos os idiomas e o iPad: [todas as capturas de tela](https://blog.cluas.eu.org/moshpit/shots/#pt-BR).

## Jogo limpo

- Compra única. Sem assinatura, sem conta, sem recursos divididos em planos.
- Nada é coletado: sem analytics, sem rastreamento. As chaves ficam nas Chaves do iOS, e o tráfego vai só para os seus próprios servidores, mais um relay de push que só vê texto cifrado.
- O Moshpit nunca instala nada no seu host sem perguntar, nunca cria sessões pelas suas costas e, quando passa a operar em modo degradado, avisa em vez de trocar de transporte em silêncio.

## Como é feito por dentro

A maior parte do Moshpit é Rust. O núcleo fica em `crates/`: o transporte SSH (`ssh`, sobre o russh), o protocolo do Mosh (`mosh`), a camada de sessão que conecta, reconecta e retoma (`session`), o modo de controle do tmux (`tmux`) e o herdr (`herdr`), o emulador de terminal (`term`, nosso fork do wezterm em `vendor/wezterm`), o que o app guarda (`store`), a máquina de estados dos agentes (`agents`), o companheiro (`pet`) e a configuração do host com o pareamento de push (`hostcare`, `install`, `seal`). As telas também são escritas em Rust, com [NSH](https://nsh.dev), em `ui/`, e os modelos delas ficam em `crates/app`.

No iOS, uma camada fina de Swift hospeda tudo isso por meio de uma única biblioteca estática (`crates/ffi`): a estrutura do app, o desenho em Metal e a entrada de texto da view do terminal, as Chaves, o ditado (Qwen3-ASR no onnxruntime do sherpa-onnx, Whisper no WhisperKit, os mecanismos de fala da Apple) e as partes que o iOS guarda para si (a Atividade ao Vivo e os widgets, o serviço de notificação que abre as notificações push por meio de `crates/seal-ffi`, a extensão de compartilhamento). Um app Android compartilha o mesmo núcleo e as mesmas telas (`crates/android`, `android/`); ele está em desenvolvimento e não foi lançado.

<details>
<summary><b>Estrutura do repositório</b></summary>

<br>

| Caminho | O que tem lá |
| :-- | :-- |
| `crates/` | O núcleo em Rust, um crate por responsabilidade: `ssh`, `mosh`, `session`, `tmux`, `herdr`, `term`, `stream` (os bytes de um painel entre o núcleo e a view do terminal), `store`, `agents`, `pet` (o companheiro), `seal` (o envelope da notificação push), `seal-ffi` (a única chamada do serviço de notificação ao `seal`), `hostcare`, `install`, `app` (os modelos das telas), `runtime`, `copy` (as regras que o texto em chinês segue), `ffi` (a biblioteca estática que o iOS linka) e `android` (a metade em Rust do app Android) |
| `ui/` | As telas, escritas em Rust com NSH e compartilhadas por iOS e Android |
| `vendor/wezterm/` | Nosso fork do emulador de terminal do wezterm, por trás de `crates/term`; [docs/WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md) lista o que ele muda |
| `Moshpit/` | A camada iOS: a estrutura do app, o lado Swift de cada tela, o desenho em Metal e a entrada de texto da view do terminal, as Chaves, o ditado (Qwen3-ASR, Whisper, mecanismos da Apple) |
| `Extensions/` | As três extensões do app: `MoshpitIsland/` (Atividade ao Vivo e widget da Tela Bloqueada, Vibe Island), `MoshpitPush/` (serviço de notificação que decifra as notificações push no aparelho), `MoshpitShare/` (entrega uma imagem ao painel de um agente) |
| `Packages/MoshpitKit/` | Pacote Swift local que o app e as três extensões linkam: logging, o payload da Atividade ao Vivo, o envelope da notificação push criptografada e o armazenamento do pareamento |
| `android/` | O app Android (Gradle), em desenvolvimento |
| `Tests/` | `MoshpitTests/` testes unitários, `MoshpitUITests/` testes de UI |
| `push-relay/` | O relay em Go, sem estado, entre o seu host e o APNs, com o manifesto de deploy em `deploy/` |
| `docs/` | [ARCHITECTURE.md](docs/ARCHITECTURE.md), [PUSH.md](docs/PUSH.md) (o protocolo de push de ponta a ponta), [WEZTERM-PATCHES.md](docs/WEZTERM-PATCHES.md), `nsh/` (notas de design sobre o núcleo em Rust e as telas em NSH), outras notas de design |
| `scripts/` | `rust/` o build da biblioteca iOS e os bindings Swift dela, `host/` os scripts de shell instalados no seu host de desenvolvimento, `gen/` geradores, `verify/` verificações de ponta a ponta, `spikes/` experimentos, `wezterm/` as importações do upstream do fork, `release/` ferramentas de archive |

</details>

Por padrão, o app usa o relay do mantenedor; [docs/PUSH.md](docs/PUSH.md) explica o que o relay consegue e não consegue ver e como rodar o seu próprio.

## Compilar do código-fonte

Você precisa de um macOS com Xcode 27 (o app roda a partir do iOS 18), do [XcodeGen](https://github.com/yonaskolb/XcodeGen) via `brew install xcodegen` e do Rust instalado pelo [rustup](https://rustup.rs). O repositório fixa a toolchain em `rust-toolchain.toml`, então o rustup instala a versão certa no primeiro uso.

```sh
git clone https://github.com/Cluas/moshpit.git
cd moshpit
xcodegen generate        # o project.yml é a fonte da verdade; o .xcodeproj é gerado a partir dele
open Moshpit.xcodeproj
```

Escolha o scheme **Moshpit** e um simulador e aperte ⌘R. A fase “Build Rust” do target do app roda `scripts/rust/build-ios.sh`, que compila `crates/ffi` em `libmoshpit_ffi.a` para o simulador ou para o aparelho e, na primeira vez, adiciona o target Rust necessário (`aarch64-apple-ios-sim`, `aarch64-apple-ios`). Builds de Debug usam o perfil `app-test` do cargo; os de Release, `app`. O primeiro build compila todo o grafo Rust e demora um pouco; os seguintes são incrementais. Os pacotes Swift também são resolvidos no primeiro build.

Um build para o aparelho precisa do seu Team ID em `Signing.xcconfig`; o comentário nesse arquivo e o [docs/install-free-account.md](docs/install-free-account.md) explicam o passo a passo, inclusive com um Apple ID gratuito.

### Testes

O lado Rust, do jeito que o CI roda:

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test -p moshpit-term        # um crate só
```

O app iOS, do jeito que o CI roda na imagem do Xcode 27:

```sh
scripts/rust/test-build-ios.sh    # confere os perfis e os links da fase Rust, sem compilar
xcodebuild test -project Moshpit.xcodeproj -scheme Moshpit \
  -destination 'platform=iOS Simulator,name=iPhone 18 Pro' CODE_SIGNING_ALLOWED=NO
```

Algumas suítes são puladas num build sem assinatura (precisam do contêiner do App Group) ou fora do simulador de iPad (atalhos de teclado físico), e alguns fluxos de UI ficam pulados até serem reescritos para as telas atuais; o log diz quais são.

O app Android compila e roda os testes com `android/ci.sh`, que baixa o que precisa (as partes do SDK, o NDK, o Gradle); basta ter um JDK 17.

## Como contribuir

Leia o [CONTRIBUTING.md](CONTRIBUTING.md) para conhecer o fluxo de trabalho e o estilo da casa, e o [SECURITY.md](SECURITY.md) antes de relatar qualquer coisa sensível. Todo mundo aqui segue o [Código de Conduta](CODE_OF_CONDUCT.md).

O Moshpit foi divulgado no [LINUX DO](https://linux.do), uma comunidade de desenvolvedores em chinês. Perguntas e ideias são tão bem-vindas lá quanto no issue tracker.

## Licença

O Moshpit é software livre, distribuído sob a [Licença Pública Geral GNU, versão 3.0](LICENSE) (GPL-3.0-only). Você pode compilar, modificar e redistribuir o programa nos termos dessa licença. A versão da App Store é publicada pelo mantenedor, que detém os direitos autorais. Componentes de terceiros mantêm as próprias licenças, listadas em [NOTICES.md](NOTICES.md).
