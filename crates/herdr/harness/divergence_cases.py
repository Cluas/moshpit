#!/usr/bin/env python3
"""Writes tests/fixtures/corpus/divergences.txt: minimal cases for every fix
in harness/divergences.json, each line `<fix id> <case>`.

    divergence_cases.py <out>
"""
import sys

from corpus import TWO_WORKSPACES, retarget_cases, x


def utf16le(text):
    return text.encode("utf-16-le")


CASES = [
    # A shell command injection through the quoting every command line uses.
    ("quote-unicode", f"quote {x(chr(120) + chr(39) + chr(0x301) + '; echo INJECTED; ' + chr(39))}"),
    ("quote-unicode", f"quote {x(chr(120) + chr(39) + chr(0xfe0f) + '; touch /tmp/pwned; ' + chr(39))}"),
    ("quote-unicode", f"quote {x('a' + chr(39) + chr(0x1f3fb) + 'b')}"),
    ("quote-unicode", f"ctl.cmd {x(TWO_WORKSPACES)} - ren-session {x('w1')} {x('it' + chr(39) + chr(0x301) + '; rm -rf ~; ' + chr(39))}"),
    ("quote-unicode", f"ctl.cmd {x(TWO_WORKSPACES)} - new-session {x('x' + chr(39) + chr(0xfe0f) + '$(id)')}"),
    # Protocol words matched by Character.
    ("unicode-syntax", f"line {x('herdr: ' + chr(0x301) + 'server rejected handshake')}"),
    ("unicode-syntax", f"drop {x('lost connection to server' + chr(0x301) + ': x')}"),
    ("unicode-syntax", f"skew {x('server rejected handshake' + chr(0x301) + ' (version 16)')}"),
    ("unicode-syntax", f"mismatch {x(chr(123) + '\"error\":{\"code\":\"protocol_mismatch\",\"message\":\"client protocol 19 is newer;' + chr(0x301) + ' run herdr server stop\"}}')}"),
    ("unicode-syntax", f"mismatch {x(chr(123) + '\"error\":{\"code\":\"protocol_mismatch\",\"message\":\"newer' + chr(0x37e) + ' upgrade; stop\"}}')}"),
    ("unicode-syntax", f"paneindex {x('x:p' + chr(0x301) + ':p3')}"),
    ("unicode-syntax", f"status {x('client:' + chr(10) + '  version:' + chr(0x301) + '0.8.0' + chr(10) + 'server:' + chr(10) + '  version: 0.9.5' + chr(10))}"),
    ("unicode-syntax", f"vcompare {x('1' + chr(0x663) + '.0')} {x('1.0')}"),
    ("unicode-syntax", f"ctl.parse {x('out' + chr(0x600) + '__moshpit_rc=0')}"),
    ("unicode-syntax", f"ctl.validate {x('/r')} {x('a ' + chr(0x301) + 'b')} {x('claude')}"),
    ("unicode-syntax", f"socket whole 0 {x(chr(0x600) + 'MOSHPIT_PUMP_READY' + chr(10))}"),
    ("unicode-syntax", f"ctl.repolines {x('/' + chr(0x301) + 'x' + chr(10) + '/y' + chr(10))}"),
    ("paste-marker-unicode", f"paste {x('ls' + chr(27) + '[201~' + chr(0x301) + chr(10) + 'echo INJECTED' + chr(10))}"),
    ("canonical-equivalence", f"ctl.repolines {x('/caf' + chr(0xe9) + chr(10) + '/cafe' + chr(0x301) + chr(10))}"),
    ("canonical-equivalence", f"ctl.cmd {x(TWO_WORKSPACES)} - manifests {x('[{\"agent\":\"e' + chr(0x301) + '\"},{\"agent\":\"' + chr(0xe9) + '\"},{\"agent\":\"f\"}]')}"),
    ("refusal-cut", f"ctl.refusal {x(('e' + chr(0x301)) * 250)} 1"),
    ("json-bom", f"line {x(chr(123) + '\"type\":\"terminal.closed\",\"reason\":\"' + chr(0xfeff) + 'bye\"}')}"),
    ("json-bom", f"json {x(chr(123) + '\"' + chr(92) + 'ufefftype\":\"terminal.frame\",\"type\":1}')}"),
    ("json-bom", f"snapshot {x(chr(123) + '\"workspaces\":[{\"workspace_id\":\"' + chr(0xfeff) + 'w1\",\"label\":\"' + chr(0xfeff) + '\"}]}')}"),
    ("json-encoding-sniff", f"json {x(utf16le(chr(123) + '\"a\":1}'))}"),
    ("json-encoding-sniff", f"line {x(utf16le(chr(123) + '\"type\":\"moshpit.ready\"}'))}"),
    ("json-decimal", f"json {x(chr(123) + '\"v\":100000000000000000.5}')}"),
    ("json-decimal", f"json {x(chr(123) + '\"v\":0.99999999999999999999999999999999999999999}')}"),
    ("json-decimal", f"line {x(chr(123) + '\"type\":\"terminal.frame\",\"seq\":1,\"width\":100000000000000000.5,\"height\":24,\"full\":1.0000000000000000000000000000000000000001,\"bytes\":\"aGk=\"}')}"),
    ("base64-quirks", f"line {x(chr(123) + '\"type\":\"terminal.frame\",\"seq\":1,\"width\":80,\"height\":24,\"full\":true,\"bytes\":\"====\"}')}"),
    ("base64-quirks", f"line {x(chr(123) + '\"type\":\"terminal.frame\",\"seq\":2,\"width\":80,\"height\":24,\"full\":false,\"bytes\":\"AAAAAA=Q\"}')}"),
    ("size-clamp", f"boot {x('w1:p1')} 0 0 - plain {x('6F9619FF-8B86-D011-B42D-00C04FC964FF')}"),
    ("size-clamp", f"start {x('w1:p1')} 0 -1 - plain"),
    ("size-clamp", "resize 70000 24"),
    ("size-clamp", "scroll 1 70000 wheel"),
    ("size-clamp", f"retarget {x('w1:p1')} 70000 1 plain"),
    ("paste-end-marker", f"paste {x('a' + chr(27) + '[20' + chr(27) + '[201~1~b')}"),
    ("paste-end-marker", f"paste {x(chr(27) + '[20' + chr(27) + '[201~1~' + chr(10) + 'echo INJECTED' + chr(10))}"),
    ("B90", f"ctl.refusal {x('')} -"),
    ("B90", f"ctl.refusal {x('bash: herdr: command not found')} -"),
    ("manifest-range-trap", f"ctl.cmd {x(TWO_WORKSPACES)} - manifests {x('a]b[')}"),
    ("version-digits", f"vcompare {x('0.99999999999999999999')} {x('0.1')}"),
    ("version-digits", f"ctl.first {x('fix the scroll')} {x('w4:p1')} {x('0.7.99999999999999999999')}"),
    ("version-digits", f"vcompare {x('0.5' + chr(0xbd))} {x('0.5')}"),
    # Every retarget command: it says a refusal its looks never saw alive.
    # The last one is quote-unicode's for a retarget (an apostrophe a
    # combining mark rides on), here while every retarget command differs.
    *(("retarget-unseen-refusal", case) for case in retarget_cases()),
    ("retarget-unseen-refusal", f"launch.retarget 1 {x('t' + chr(39) + chr(0x301) + ';id')} {x('K')}"),
]


def main():
    with open(sys.argv[1], "w", encoding="utf-8") as f:
        f.write("\n".join(f"{fix} {case}" for fix, case in CASES) + "\n")
    print(f"{len(CASES)} divergence cases", file=sys.stderr)


if __name__ == "__main__":
    main()
