#!/usr/bin/env python3
"""Synthetic -CC streams for the G2 parser equivalence harness.

    synth.py fixtures <fixtures-dir>     the committed synthetic inputs
    synth.py fuzz <dir> <count>          seeded random streams (harness time)

`fixtures` writes:
  synthetic/recovery.bin   every recovery and malformed-line path the Swift
                           parser gets right; both parsers must agree
  divergences/<name>.bin   one minimal capture per documented fix
                           (harness/divergences.json)

`fuzz` streams mix real protocol shapes, pane-content look-alikes, invalid
UTF-8 and garbage with the documented fixes' triggers — the Swift parser
carries those fixes now: glued %begin lines with ids at the Int limits or a
non-ASCII flag, blocks of exactly 5000 lines, embedded terminators with a
longer number or a non-ASCII tail, every zoom flag, and combining characters
next to protocol syntax. The two parsers must agree on them byte for byte.
"""
import os
import random
import sys

T = 1700000000  # %begin time field
CRLF = b"\r\n"


def boot(num=100, t=T):
    return b"%%begin %d %d 0\r\n%%end %d %d 0\r\n" % (t, num, t, num)


def lines(*items):
    return b"".join(item if item.endswith(b"\n") else item + CRLF for item in items)


# MARK: - Committed fixtures

def recovery():
    out = bytearray()
    out += b"\x1bP1000p" + boot(100)
    out += lines(
        b"%session-changed $0 main",
        b"%output %0 plain",
        # Truncated-line recovery, in-block half: the block's own
        # terminator glued onto a content line, then a glued %error.
        b"%begin 1700000000 101 1",
        b"line one",
        b"content line cut short%end 1700000000 101 1",
        b"%begin 1700000000 102 1",
        b"glued error%error 1700000000 102 1",
        # Output half: a %begin with a plausible next number glued onto a
        # truncated %output opens the block it starts.
        b"%output %0 before the cut%begin 1700000001 103 1",
        b"reply of 103",
        b"%end 1700000001 103 1",
        # ...but not with a stale or far-off number, a far-off time, a
        # two-digit flag, extra fields, invalid UTF-8, or at payload start.
        b"%output %0 quoting an old dump %begin 1700000000 5 1",
        b"%output %0 future %begin 1700000001 400 1",
        b"%output %0 far time %begin 1600000000 104 1",
        b"%output %0 two flags %begin 1700000001 104 10",
        b"%output %0 extra %begin 1700000001 104 1 x",
        b"%output %0 bad utf8 %begin 1700000001 104 \xff",
        b"%output %0 %begin 1700000001 104 1",
        # Terminators: extra fields still close, foreign ids are content,
        # a bare keyword closes, a double space is content.
        b"%begin 1700000002 104 1",
        b"%end 1700000002 105 1",
        b"%end  1700000002 104 1",
        b"%end 1700000002 104 1 extra",
        b"%begin 1700000002 105 1",
        b"%end",
        b"%begin 1700000002 106 1",
        b"",
        b"%error",
        # Stray terminators outside a block.
        b"%end 1700000002 106 1",
        b"%end",
        b"%error 7 8 1",
        b"%error",
        # Malformed notifications.
        b"%window-add ",
        b"%window-close ",
        b"%window-renamed @1",
        b"%session-changed ",
        b"%session-window-changed $1",
        b"%session-window-changed $1 ",
        b"%window-pane-changed @1",
        b"%pause ",
        b"%continue ",
        b"%layout-change @1",
        b"%layout-change ",
        b"%begin x y",
        b"%begin 1",
        b"%output %0",
        b"%output  leading space",
        b"%output \xff\xfe bad pane id",
        # Odd but valid notifications.
        b"%session-window-changed  $1 @2",
        b"%session-window-changed $1  @2",
        b"%window-pane-changed @1 %2 extra",
        b"%layout-change @1  b25d,80x24,0,0,0  b25d,80x24,0,0,0  *Z",
        b"%layout-change @1 b25d,80x24,0,0,0 b25d,80x24,0,0,0 ",
        b"%layout-change @1 b25d,80x24,0,0,0 *Z extra",
        b"%window-renamed @1 ",
        b"%window-renamed @2 two  spaces ",
        "%window-renamed @3 汉字窗口 😀".encode(),
        "%session-changed $4 会话 👨‍👩‍👧".encode(),
        b"%client-detached",
        b"%client-detached /dev/ttys001",
        b"%client-detachedX",
        b"%pause %4",
        b"%continue %4",
        # Junk before a keyword: kept for keywords only, from the first %.
        b"garbage%window-add @3",
        b"\x1bP1000p%layout-change @1 b25d,80x24,0,0,0 b25d,80x24,0,0,0 *",
        b"50% done %window-add @4",
        b"no percent at all",
        b"\xef\xbb\xbf%window-add @5",
        b"%this-is-from-the-future @0 payload",
        b"%sessions-changed",
        b"%extended-output %0 0 : hidden",
        # Notifications about the rest of the server (tmux 3.6a), valid,
        # odd and cut short.
        b"%session-renamed $1 renamed",
        b"%session-renamed $1 two  spaces ",
        b"%session-renamed $1 ",
        b"%session-renamed  $1 x",
        b"%session-renamed $1",
        b"%sessions-changed extra",
        b"%sessions-changedX",
        b"%unlinked-window-add @5",
        b"%unlinked-window-add ",
        b"%unlinked-window-close @5",
        b"%unlinked-window-close ",
        b"%unlinked-window-renamed @5 my window ",
        b"%unlinked-window-renamed @5 ",
        b"%unlinked-window-renamed @5",
        b"%unlinked-window-renamed  name",
        b"%client-session-changed /dev/ttys024 $1 my project",
        b"%client-session-changed client-4242 $0 ",
        b"%client-session-changed /dev/ttys024 $1",
        b"%client-session-changed  $1 x",
        b"%client-session-changed /dev/ttys024  x",
        b"%client-session-changed",
        b"junk%unlinked-window-add @6",
        "%unlinked-window-renamed @7 \u0301x".encode(),
        "%client-session-changed /dev/ttys1 $2 会话 👨‍👩‍👧".encode(),
        # Line endings: LF only, one CR stripped of two, a lone CR.
        b"%window-add @6\n",
        b"%window-add @7\r\r\n",
        b"\r\n",
        b"\n",
        # Octal edge cases and raw high bytes in %output.
        b"%output %1 \\134\\\\\\777\\400 \\12",
        b"%output %1 \\0\\01\\012\\x \\",
        b"%output %1 \xe6\xb5\x8b\xe8\xaf",
        b"%output %1 \x95\xf0\x9f\x98\x80",
        b"%output %1 nul\\000byte \x00 raw nul",
        # Block content: invalid UTF-8, BOMs, NUL, controls, look-alikes.
        b"%begin 1700000003 107 1",
        b"a\xffb\xc0\x80c\xed\xa0\x80d\xf4\x90\x80\x80e\xe2\x94f\xf0\x9f\x98g\xc2",
        b"\xef\xbb\xbfbom then text",
        b"\xef\xbb\xbf\xef\xbb\xbftwo boms",
        b"\xef\xbb\xbfbom then bad \xff",
        b"\xef\xbb\xbf",
        b"mid\xef\xbb\xbfbom",
        b"nul\x00inside",
        b"\x1b[31mred\x1b[0m \x07 \x7f",
        b"%output %0 not a notification",
        b"%exit",
        b"%begin 9 9 9",
        b"%end 9 9 9",
        b"%end 1700000003 107",
        # %exit re-arms boot swallowing; the next session's banner follows
        # an ST and a fresh DCS on the same line.
        b"%exit",
        b"%exit ",
        b"\x1b\\\x1bP1000p%begin 1700000004 200 0",
        b"%end 1700000004 200 0",
        b"%exit server exited",
        b"%begin 1700000004 201 1",
        b"swallowed as the next boot reply",
        b"%end 1700000004 201 1",
        b"%begin 1700000004 202 1",
        b"delivered",
        b"%end 1700000004 202 1",
        b"x%exit",
        b"%begin 1700000004 203 1",
        b"swallowed again after the junk-prefixed exit",
        b"%end 1700000004 203 1",
        b"%window-add @8",
    )
    # Runaway block: the terminator never comes; the 5001st line forces the
    # close and is dispatched again, outside the block.
    out += b"%begin 1700000005 300 1\r\n" + CRLF * 5000 + b"%window-add @9\r\n"
    out += b"%begin 1700000005 301 1\r\n" + CRLF * 5001 + b"%end 1700000005 301 1\r\n"
    out += lines(b"%window-close @9")
    # A long line, and a partial line left at the end of the stream.
    out += b"%output %2 " + b"long " * 400 + CRLF
    out += b"%output %2 never terminated"
    return bytes(out)


def divergences():
    """One minimal capture per documented fix (harness/divergences.json)."""
    zoom = [b"-Z", b"*MZ", b"Z", b"#*Z", b"*Z", b"*", b"-"]
    layout = b"9bb2,80x24,0,0{26x24,0,0,0,26x24,27,0,1,26x24,54,0,2}"
    return {
        "overflow-trap": boot(100) + lines(
            b"%output %0 x%begin -9223372036854775808 101 1",
            b"%output %0 after",
        ),
        "block-cap-terminator": boot(100)
        + b"%begin 1700000000 101 1\r\n"
        + b"\r\n" * 5000
        + lines(b"%end 1700000000 101 1", b"%window-add @1"),
        "terminator-number-boundary": boot(9) + lines(
            b"%begin 1700000000 10 1",
            b"seen in a log: %end 1700000000 105 1",
            b"bar",
            b"%end 1700000000 10 1",
            b"%window-add @1",
        ),
        "zoom-flags": boot(100) + lines(
            *[b"%layout-change @1 " + layout + b" b25d,80x24,0,0,0 " + flags for flags in zoom]
        ),
        "unicode-delimiter": boot(100) + lines(
            "%window-renamed @1 \u0301x".encode(),
            "%session-changed $1 \u0301s".encode(),
            "junk%\u0301window-add @1 %window-add @2".encode(),
        ),
        "unicode-flag": boot(100) + lines(
            "%output %0 x%begin 1700000000 101 \u4e94".encode(),
            b"%begin 1700000000 101 1",
            b"real reply",
            b"%end 1700000000 101 1",
        ),
        "unicode-tail": boot(9) + lines(
            b"%begin 1700000000 10 1",
            "foo%end 1700000000 10 \u0663".encode(),
            b"bar",
            b"%end 1700000000 10 1",
        ),
    }


# MARK: - Fuzz

WORDS = ["main", "logs", "build", "vim", "zsh", "bash", "claude", "汉字", "会话", "终端",
         "😀", "👨‍👩‍👧", "┌─┐", "été", "naïve", "x-y_z", "a.b", "1234"]
CONTENT = ["plain content", "build is 80%done", "x%window y", "100%end of run",
           "%end 5 6 0", "%error 1 2 1", "%begin 3 4 1", "%output %1 fake", "%exit",
           "%window-add @9", "测试中文 ┌─┐ │x│", "😀👍🏽 emoji", "\x1b[1;31mred\x1b[0m",
           "tab\there", "   leading spaces", ""]
# Lines whose meaning used to depend on Characters (harness/divergences.json,
# unicode-syntax): a combining mark right after protocol syntax.
COMBINING = ["%window-renamed @1 \u0301x", "%session-changed $1 \u0301s",
             "junk%\u0301window-add @1 %window-add @2", "%window-add \u0301@3",
             "%layout-change @1 \u0301b25d,80x24,0,0,0 *Z", "%pause \u0301%1",
             "%session-window-changed $1 \u0301@2", "%exit \u0301", "%client-detached\u0301",
             "%unlinked-window-renamed @1 \u0301x", "%session-renamed $1 \u0301s",
             "%client-session-changed /dev/ttys1 \u0301$1 x"]
# Notifications about other sessions, windows and clients, as tmux 3.6a sends
# them and cut short.
SERVER = ["%sessions-changed", "%unlinked-window-add @3", "%unlinked-window-close @3",
          "%unlinked-window-renamed @3 x y", "%session-renamed $1 x", "%session-renamed $1 a|b c",
          "%client-session-changed /dev/ttys1 $1 x", "%client-session-changed client-9 $2 a b",
          "%unlinked-window-add ", "%unlinked-window-renamed @3", "%session-renamed $1",
          "%client-session-changed /dev/ttys1 $1", "%client-session-changed  $1 x"]
# `%pane-mode-changed` is parsed since B614 (the mosh path's copy-mode).
UNKNOWN = ["%subscription-changed x $1 @1 1 %1 : v",
           "%extended-output %0 0 : abc", "%paste-buffer-changed buffer0", "%message hi",
           "%config-error x"]


def fuzz_stream(seed, target=60_000):
    rnd = random.Random(seed)
    t, num = T + seed * 1000, 100 + seed
    out = bytearray(b"\x1bP1000p")

    def end():
        r = rnd.random()
        return b"\r\n" if r < 0.85 else b"\n" if r < 0.97 else b"\r\r\n"

    def line(text):
        out.extend(text if isinstance(text, bytes) else text.encode())
        out.extend(end())

    def word():
        return rnd.choice(WORDS)

    def raw_bytes(n):
        # Never '%' or '\n': no accidental protocol syntax, no line breaks.
        return bytes(rnd.choice([b for b in range(256) if b not in (0x25, 0x0A)]) for _ in range(n))

    def payload():
        parts = []
        for _ in range(rnd.randint(0, 12)):
            r = rnd.random()
            if r < 0.4:
                parts.append(rnd.choice(["hello ", "world", "seq 1 2 3", "$ ", "…", "汉字", "😀"]).encode())
            elif r < 0.6:
                parts.append(b"\\%03o" % rnd.choice([0, 7, 8, 10, 13, 27, 92, 127, 255, rnd.randrange(256)]))
            elif r < 0.75:
                parts.append(bytes(rnd.randrange(0x80, 0x100) for _ in range(rnd.randint(1, 4))))
            elif r < 0.85:
                parts.append(rnd.choice([b"\\", b"\\\\", b"\\1", b"\\12", b"\\8", b"\\778"]))
            else:
                parts.append("─│┌┐└┘".encode()[: rnd.randint(1, 9)])
        return b"".join(parts)

    def session_boot():
        nonlocal num
        num += 1
        line(f"%begin {t} {num} 0")
        if rnd.random() < 0.3:
            line("can't find session: nope")
            line(f"%error {t} {num} 0")
            line("%exit")
            out.extend(b"\x1b\\\x1bP1000p")
            num += 1
            line(f"%begin {t} {num} 0")
        line(f"%end {t} {num} 0")

    session_boot()
    while len(out) < target:
        r = rnd.random()
        if r < 0.35:
            pane = f"%{rnd.randrange(4)}".encode()
            body = payload()
            reply = None
            if rnd.random() < 0.05:
                # A glued %begin: plausible (opens a block) or stale, its id
                # sometimes at the Int limits and its flag not always an
                # ASCII digit (overflow-trap, unicode-syntax).
                nxt = num + rnd.choice([1, 2, 150, 300, -5])
                cid = rnd.choice([t, t, t + 1, t + 100000, -9223372036854775808, 9223372036854775807])
                flag = rnd.choice(["0", "1", "1", "\u4e94", "\u0663", "1\u0301"])
                body += f"%begin {cid} {nxt} {flag}".encode()
                if nxt > num and nxt - num <= 200:
                    num = max(num, nxt)
                    reply = (cid, nxt, flag)
            line(b"%output " + pane + b" " + body)
            if reply and rnd.random() < 0.8:
                # The reply of a real glued %begin follows it.
                line("glued reply")
                line(f"%end {reply[0]} {reply[1]} {reply[2]}")
        elif r < 0.6:
            num += 1
            t += rnd.choice([0, 0, 1])
            line(f"%begin {t} {num} 1")
            if rnd.random() < 0.01:
                # Exactly the runaway cap (block-cap-terminator).
                out.extend(b"x\r\n" * 5000)
            for _ in range(rnd.choice([0, 1, 2, 3, 5, 8, 20, 60])):
                c = rnd.random()
                if c < 0.55:
                    line(rnd.choice(CONTENT))
                elif c < 0.75:
                    line(raw_bytes(rnd.randint(0, 30)))
                elif c < 0.9:
                    line(f"%end {t + rnd.randint(-3, 3)} {num + rnd.choice([-1, 1, 2])} 1")
                else:
                    # The open block's own ids with a longer number or a
                    # non-ASCII flag after them (terminator-number-boundary,
                    # unicode-syntax): content, not the terminator.
                    line(rnd.choice(CONTENT) + rnd.choice([
                        f"%end {t} {num}{rnd.randrange(10)} 1", f"%error {t} {num}{rnd.randrange(10)}",
                        f"%end {t} {num} \u0663", f"%end {t} {num} 1\u0301"]))
            c = rnd.random()
            if c < 0.76:
                line(f"%end {t} {num} 1")
            elif c < 0.87:
                line(f"%error {t} {num} 1")
            elif c < 0.95:
                line(rnd.choice(CONTENT) + f"%end {t} {num} 1")
            elif c < 0.99:
                line("%end")
            # else: the terminator is lost; the block runs on.
        elif r < 0.8:
            kind = rnd.randrange(13)
            w, p, s = f"@{rnd.randrange(6)}", f"%{rnd.randrange(6)}", f"${rnd.randrange(3)}"
            if kind == 0:
                line(f"%window-add {w}")
            elif kind == 1:
                line(f"%window-close {w}")
            elif kind == 2:
                line(f"%window-renamed {w} {word()} {word() if rnd.random() < 0.3 else ''}".rstrip(" ") or "x")
            elif kind == 3:
                line(f"%session-changed {s} {word()}" if rnd.random() < 0.8 else f"%session-changed {s}")
            elif kind == 4:
                line(f"%session-window-changed {s} {w}")
            elif kind == 5:
                line(f"%window-pane-changed {w} {p}")
            elif kind == 6:
                flags = rnd.choice(["*Z", "*", "-", "", "-Z", "Z", "*MZ", "#*Z", "!-Z", "~Z", "*M"])
                line(f"%layout-change {w} b25d,80x24,0,0,0 b25d,80x24,0,0,0 {flags}".rstrip(" ")
                     if rnd.random() < 0.8 else f"%layout-change {w} b25d,80x24,0,0,0 {flags}".rstrip(" "))
            elif kind == 7:
                line(rnd.choice([f"%pause {p}", f"%continue {p}"]))
            elif kind == 8:
                line(rnd.choice(["%client-detached", "%client-detached /dev/ttys3"]))
            elif kind == 9:
                line(rnd.choice(UNKNOWN + SERVER))
            elif kind == 10:
                line(rnd.choice(["%window-add ", "%pause ", "%window-renamed @1", "%begin x 1",
                                 "%output %0", "%end 1 2 3", "%layout-change @1"]))
            elif kind == 11:
                line(rnd.choice(["%exit", "%exit detached", "%exit server exited"]))
                session_boot()
            else:
                line(rnd.choice(COMBINING))
        else:
            g = rnd.random()
            if g < 0.4:
                line(raw_bytes(rnd.randint(0, 40)))
            elif g < 0.6:
                line(rnd.choice(["", "\r", "no percent", "\x1bP1000p"]) + f"%window-add @{rnd.randrange(6)}")
            elif g < 0.8:
                line(rnd.choice(["Last login: Mon", "sh-3.2$ tmux -CC attach", "50% loaded", "x%exit"]))
            else:
                line(b"")
    out.extend(b"%exit\r\n\x1b\\")
    return bytes(out)


def main(argv):
    if len(argv) == 3 and argv[1] == "fixtures":
        root = argv[2]
        os.makedirs(os.path.join(root, "synthetic"), exist_ok=True)
        os.makedirs(os.path.join(root, "divergences"), exist_ok=True)
        with open(os.path.join(root, "synthetic", "recovery.bin"), "wb") as f:
            f.write(recovery())
        for name, data in divergences().items():
            with open(os.path.join(root, "divergences", f"{name}.bin"), "wb") as f:
                f.write(data)
    elif len(argv) == 4 and argv[1] == "fuzz":
        os.makedirs(argv[2], exist_ok=True)
        for seed in range(1, int(argv[3]) + 1):
            with open(os.path.join(argv[2], f"fuzz-{seed}.bin"), "wb") as f:
                f.write(fuzz_stream(seed))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
