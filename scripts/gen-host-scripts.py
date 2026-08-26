#!/usr/bin/env python3
"""Regenerate Moshpit/Services/Install/HostScripts.swift from the canonical shell files.

Two copies of each host script exist because the app cannot read a repo file at
runtime on a phone: the file under scripts/ is canonical (and is what the shell
and Go tests actually execute), the Swift literal is what ships. This keeps them
byte-identical; HostScriptsTests fails if anyone edits one alone.

Run from the repo root:  scripts/gen-host-scripts.py
"""
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "Moshpit" / "Services" / "Install" / "HostScripts.swift"

SCRIPTS = [
    ("stamp", "scripts/moshpit-stamp.sh",
     "What an agent's lifecycle hook calls: stamps the pane, hands attention/done to the sender."),
    ("sender", "scripts/moshpit-push.sh",
     "Seals one agent status and hands it to the push relay."),
]

HEADER = '''import Foundation

/// The shell Moshpit installs on a dev host, verbatim.
///
/// Each literal is byte-for-byte its file under `scripts/`, which is the
/// canonical copy and the one the shell and Go tests actually execute. Two
/// copies exist only because the app cannot read a repo file at runtime on a
/// phone; `HostScriptsTests` fails the moment they diverge.
///
/// Regenerate after editing any of the shell files:
///
///     scripts/gen-host-scripts.py
///
/// Do not hand-edit the literals.
///
/// These are delivered as FILES over an exec channel — not pasted into a shell —
/// so unlike the flow this replaces they carry their own comments, may contain
/// single quotes, and need no escaping of any kind. A user who goes looking at
/// `~/.moshpit/` finds readable programs.
enum HostScripts {
'''

FOOTER = '''}
'''


def literal(name, body, doc):
    if '"""' in body or "\\#(" in body:
        sys.exit(f"error: {name} contains a sequence that breaks a Swift raw literal")
    return (f'\n    /// {doc}\n'
            f'    static let {name} = #"""\n'
            f'{body.rstrip(chr(10))}\n'
            f'"""#\n')


parts = [HEADER]
for name, rel, doc in SCRIPTS:
    parts.append(literal(name, (ROOT / rel).read_text(), doc))

parts.append('''
    /// Content digest of each script, as the manifest records it. Computed from
    /// the literal rather than stored, so it cannot fall out of date.
    static func digest(of component: InstallComponent) -> String? {
        switch component {
        case .stamp:  return ContentDigest.of(stamp)
        case .sender: return ContentDigest.of(sender)
        case .hooks, .pairing: return nil
        }
    }

    /// Body of a component that is a plain file, if it has a fixed one.
    static func body(of component: InstallComponent) -> String? {
        switch component {
        case .stamp:  return stamp
        case .sender: return sender
        case .hooks, .pairing: return nil
        }
    }
''')
parts.append(FOOTER)
OUT.write_text("".join(parts))
print(f"wrote {OUT.relative_to(ROOT)}")
