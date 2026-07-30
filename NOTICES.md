# Third-Party Notices

Ringdown is licensed under the MIT License (see [LICENSE](LICENSE)). It
incorporates the following third-party components, each under its own license.

## Swift Package Dependencies

### SwiftTerm (Cluas fork)

- Source: https://github.com/Cluas/SwiftTerm
- Upstream: https://github.com/migueldeicaza/SwiftTerm
- License: MIT

Ringdown depends on a fork of SwiftTerm maintained at
`github.com/Cluas/SwiftTerm`, forked from `migueldeicaza/SwiftTerm`. The fork
carries a small set of terminal-rendering and hyperlink/IME fixes (see the
fork's commit history and the annotated `SwiftTerm` entry in `project.yml`).
Both the upstream project and the fork are distributed under the MIT License.

### Citadel

- Source: https://github.com/orlandos-nl/Citadel
- License: MIT

Citadel provides the SSH client implementation used by Ringdown.

## Bundled Fonts

Ringdown bundles several programmer monospace fonts, selectable under
Settings ▸ Display ▸ Font. Each font is redistributable and retains its own
license. The per-font license and source table is maintained in
[`Ringdown/Resources/Fonts/LICENSES.md`](Ringdown/Resources/Fonts/LICENSES.md) and
is incorporated here by reference.

In summary, the bundled fonts are covered by the SIL Open Font License 1.1
(JetBrains Mono, Maple Mono, Fira Code, Source Code Pro, IBM Plex Mono,
Anonymous Pro) and the MIT-style Hack Open Font License (Hack). See the
referenced file for full attribution, source URLs, and license text pointers.

## Clean-Room Disclaimer — Mosh State Synchronization Protocol

The Swift sources under `Ringdown/Services/Mosh/` (`MoshBootstrap.swift`,
`MoshCompression.swift`, `MoshCrypto.swift`, `MoshTransport.swift`,
`MoshWire.swift`, `OCB3.swift`) are an **independent, clean-room Swift
reimplementation** of mosh's State Synchronization Protocol (SSP) and its
associated wire format and cryptography.

This implementation was written from the public description of the protocol —
its published specification and RFC-style documented on-the-wire behavior — for
the sole purpose of interoperating with mosh servers. It is **not** derived
from, and does not copy or translate, mosh's own GPL-3.0-licensed C++ source
code.

Because it is an original work implementing an interoperable protocol rather
than a derivative of the mosh codebase, this code carries **no GPL
obligations** and is distributed under Ringdown's MIT License along with the rest
of the project.

"mosh" is a project of Keith Winstein and contributors; the name is used here
only to identify the protocol being interoperated with.
