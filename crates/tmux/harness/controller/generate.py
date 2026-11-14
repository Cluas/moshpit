#!/usr/bin/env python3
"""Write the SwiftPM package of the Swift side of the controller harness.

    generate.py <package dir>

The package compiles the app's tmux controller for macOS:

  Sources/MoshpitKit/   Packages/MoshpitKit/.../Log.swift, unchanged
  Sources/Controller/
    TmuxSessionController.swift   the app's, with the substitutions below
    App/*.swift                   app sources compiled unchanged
    Extract.swift                 verbatim declarations cut from SessionHub.swift
                                  and TmuxTransport.swift (files that drag in
                                  Citadel / UIKit)
    Stubs.swift, Harness.swift, main.swift   harness/controller/swift/

What is cut out of TmuxSessionController.swift, and why (each count is
checked: a change in the app's source fails the build instead of testing
stale code):

  import SwiftTerm / import UIKit      the views are stubs (Stubs.swift):
                                       TerminalView, the coordinator, fonts,
                                       themes, cursors, UIDevice
  Task.sleep( -> HarnessClock.sleep(   virtual time: a script's `wait` fires
  Date() -> HarnessClock.date()        the timers in deadline order and the
  ContinuousClock.now -> HarnessClock  clocks read the script's clock
  handle(_:) gets a `defer` counter    so the runner knows when every event
                                       the bytes made has been handled
  Dictionary/Set iteration in id order Swift leaves dictionary order
                                       unspecified (and different in every
                                       process); the port walks ids in the
                                       order tmux made them, so the harness
                                       pins Swift to that one of its orders

What is added, and why (the only logic that is not the app's today):

  countNonControlClients               the app dropped it (93a6c82a: only its
                                       tests still called it); the Rust
                                       controller keeps
                                       `count_non_control_clients`, and five
                                       scripts ask (`count`). Put back as the
                                       app had it, on the controller's own
                                       `list-clients` path, right before
                                       `sendCommand`.

Nothing else changes: every other line of logic is the app's.
"""
import os
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(HERE))))
CONTROLLER = "Moshpit/Services/Tmux/TmuxSessionController.swift"

# The app's `countNonControlClients` until 93a6c82a, word for word.
COUNT_NON_CONTROL_CLIENTS = """\
    func countNonControlClients(_ completion: @escaping (Int) -> Void) {
        guard snapshot.isAttached else { completion(-1); return }
        let switches = sessionSwitches
        sendCommand("list-clients -F '\\(TmuxClientRow.format)'", waits: true) { [weak self] response in
            guard !response.isError else {
                completion(-1)
                return
            }
            let clients = response.lines.compactMap(TmuxClientRow.init).filter { !$0.isControl }
            // The rows still say where an already known renderer sits.
            self?.noteRendererRows(clients, switches: switches)
            completion(clients.count)
        }
    }

"""

SUBSTITUTIONS = [
    ("import UIKit\n", "", 1),
    ("Task.sleep(", "HarnessClock.sleep(", 18),
    ("Date()", "HarnessClock.date()", 13),
    ("ContinuousClock.now", "HarnessClock.continuousNow", 2),
    ("    private func handle(_ event: ControlEvent) {\n",
     "    private func handle(_ event: ControlEvent) {\n        defer { HarnessProbe.handled += 1 }\n", 1),
    ("snapshot.panes.values", "snapshot.panes.harnessIdOrderedValues", 8),
    ("snapshot.windows.values", "snapshot.windows.harnessIdOrderedValues", 2),
    ("paneCoordinators.values", "paneCoordinators.harnessIdOrderedValues", 1),
    ("for win in resizedWindows {", "for win in resizedWindows.sorted(by: TmuxSnapshot.idPrecedes) {", 1),
    ("Array(pristineLayouts.keys)", "pristineLayouts.keys.sorted(by: TmuxSnapshot.idPrecedes)", 1),
    ("for paneId in paneResizeAwaitingCapture {",
     "for paneId in paneResizeAwaitingCapture.sorted(by: TmuxSnapshot.idPrecedes) {", 1),
    ("    private func sendCommand(_ command: String, waits: Bool = false,\n",
     COUNT_NON_CONTROL_CLIENTS + "    private func sendCommand(_ command: String, waits: Bool = false,\n", 1),
]

UNCHANGED = [
    "Moshpit/Services/Tmux/TmuxControlClient.swift",
    "Moshpit/Services/Tmux/TmuxTypes.swift",
    "Moshpit/Services/Tmux/TmuxClientRow.swift",
    "Moshpit/Services/Tmux/TmuxFormatFields.swift",
    "Moshpit/Services/Tmux/BellDetector.swift",
    "Moshpit/Services/MultiplexerControlling.swift",
    "Moshpit/UI/Terminal/ScrollMotion.swift",
    "Moshpit/Services/Shell.swift",
    "Moshpit/Services/BracketedPaste.swift",
]

PACKAGE = """// swift-tools-version:5.9
// Generated by crates/tmux/harness/controller/generate.py. Do not edit.
import PackageDescription

let package = Package(
    name: "ControllerHarness",
    platforms: [.macOS("15.0")],
    targets: [
        .target(name: "MoshpitKit", path: "Sources/MoshpitKit"),
        .executableTarget(name: "controller-swift", dependencies: ["MoshpitKit"],
                          path: "Sources/Controller"),
    ],
    swiftLanguageVersions: [.v5]
)
"""


def read(path):
    return open(os.path.join(REPO, path), encoding="utf-8").read()


def block(path, signature):
    """Lines from the signature line to the line closing it at its indentation."""
    lines = read(path).split("\n")
    starts = [i for i, line in enumerate(lines) if signature in line]
    if len(starts) != 1:
        sys.exit(f"generate.py: {signature!r} found {len(starts)} times in {path}")
    start = starts[0]
    indent = lines[start][: len(lines[start]) - len(lines[start].lstrip())]
    depth = 0
    for end in range(start, len(lines)):
        depth += lines[end].count("{") - lines[end].count("}")
        if depth == 0 and end > start:
            if lines[end] != indent + "}":
                sys.exit(f"generate.py: {signature!r} in {path} does not close at its indentation")
            return lines[start : end + 1], start + 1
    sys.exit(f"generate.py: {signature!r} in {path} never closes")


def substituted():
    text = read(CONTROLLER)
    for old, new, count in SUBSTITUTIONS:
        found = text.count(old)
        if found != count:
            sys.exit(f"generate.py: {old!r} found {found} times in {CONTROLLER}, expected {count}")
        text = text.replace(old, new)
    return text


def extract():
    hub = "Moshpit/Services/SessionHub.swift"
    out = [
        "// Generated by crates/tmux/harness/controller/generate.py: verbatim",
        "// declarations cut from app files the harness cannot compile whole.",
        "import Foundation",
        "",
        "enum SessionHub {",
        "    enum ActiveSession {",
    ]
    for signature in ("static func arrowKeyName(", "nonisolated static func clickBytes(",
                      "nonisolated static func wheelBytes("):
        lines, number = block(hub, signature)
        out.append(f"        // {hub}:{number}")
        out.extend(lines)
    out += ["    }", "}", ""]
    for signature in ("func withTimeoutValue<T: Sendable>(", "private final class OnceContinuation<V>"):
        lines, number = block(hub, signature)
        out.append(f"// {hub}:{number}")
        out.extend(line.replace("Task.sleep(", "HarnessClock.sleep(") for line in lines)
        out.append("")
    transport = "Moshpit/Services/Tmux/TmuxTransport.swift"
    lines, number = block(transport, "protocol TmuxTransport: Sendable {")
    out.append(f"// {transport}:{number}")
    out.extend(lines)
    out += ["", "// Dictionary order pinned to tmux's id order (see generate.py).",
            "extension Dictionary where Key == String {",
            "    var harnessIdOrderedValues: [Value] {",
            "        keys.sorted(by: TmuxSnapshot.idPrecedes).map { self[$0]! }",
            "    }", "}", ""]
    return "\n".join(out)


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    root = sys.argv[1]
    sources = os.path.join(root, "Sources")
    if os.path.isdir(sources):
        shutil.rmtree(sources)
    controller = os.path.join(sources, "Controller")
    os.makedirs(os.path.join(controller, "App"))
    os.makedirs(os.path.join(sources, "MoshpitKit"))
    with open(os.path.join(root, "Package.swift"), "w") as f:
        f.write(PACKAGE)
    shutil.copy(os.path.join(REPO, "Packages/MoshpitKit/Sources/MoshpitKit/Log.swift"),
                os.path.join(sources, "MoshpitKit", "Log.swift"))
    with open(os.path.join(controller, "TmuxSessionController.swift"), "w", encoding="utf-8") as f:
        f.write(substituted())
    for path in UNCHANGED:
        shutil.copy(os.path.join(REPO, path), os.path.join(controller, "App", os.path.basename(path)))
    with open(os.path.join(controller, "Extract.swift"), "w", encoding="utf-8") as f:
        f.write(extract())
    for name in ("Stubs.swift", "Harness.swift", "main.swift"):
        shutil.copy(os.path.join(HERE, "swift", name), os.path.join(controller, name))


main()
