#!/usr/bin/env python3
"""Generate the two tmux panes' screen content for the mosh pane-switch
render-divergence rig (scripts/capture-mosh-switch-bytes.sh).

Each pane is a static .ansi blob `cat`ed once at pane start, so tmux's model
of the pane is fully deterministic and `capture-pane -p -e` is a usable
ground truth.

Two blocks of content, both tagged per row so a per-cell mismatch report maps
straight back to the case that seeded it.

BLOCK A — standalone cases, one row each:

  01 plain ASCII + trailing spaces      baseline
  02 CJK ending EXACTLY on the last column
  03 CJK straddling the last column     (wide char can't fit → pad + wrap)
  04 inverse-video space mid-line       claude's block cursor
  05 inverse-video space right after a wide char
  06 inverse-video space AT the last column
  07 inverse-video WIDE char
  08 SGR background + EL                BCE (erase-with-background)
  09-11 box drawing                     ambiguous-width (╭ ─ │ ╰)
  12 claude's status glyphs             ⏺ ✻ ⎿ → …  (mixed W/A/N widths)
  13 emoji                              emoji-presentation width
  14 wrapped CJK+ASCII paragraph        the shape the report describes
  15 trailing spaces after CJK

BLOCK B — PAIRED rows. Both panes get a row at the same screen line whose
content is deliberately ALMOST identical, so the pane switch does not paint a
fresh screen: it makes mosh emit a MINIMAL in-place diff — cursor jumps into
the middle of a row and overwrites a handful of cells. That is where a client
without its own framebuffer can quietly disagree with mosh-server:

  20 narrow char landing on a wide char's FIRST half
  21 narrow char landing on a wide char's SECOND half
  22 wide char landing on top of two narrow chars
  23 an inverse-video run that SHRINKS (cells must lose their reverse bit)
  24 an inverse-video run that GROWS
  25 an inverse tail running to EOL vs a plain tail (erase-under-reverse)
  26 a coloured-bg tail to EOL vs a plain tail (erase-under-bg)
  27 a row whose wrap parity flips (wide char at the last column vs not)
  28 inverse block cursor moving one column (claude's caret stepping)
  29 CJK line differing only in its final wide char

Widths follow wcwidth-under-a-UTF-8-locale rules (East Asian W/F = 2,
everything else including AMBIGUOUS = 1) — the same rule mosh-server's
emulator uses, so "exactly on the last column" really is exact server-side.
Any client that disagrees about a width is precisely the bug under test.
"""
import argparse
import unicodedata

ESC = "\033"
INV = ESC + "[7m"
OFF = ESC + "[0m"


def cwidth(ch: str) -> int:
    if unicodedata.combining(ch):
        return 0
    if unicodedata.east_asian_width(ch) in ("W", "F"):
        return 2
    return 1


def swidth(s: str) -> int:
    """Display width of a string, ignoring any escape sequences it contains."""
    total = 0
    i = 0
    while i < len(s):
        if s[i] == ESC:
            j = i + 2
            while j < len(s) and not ("@" <= s[j] <= "~"):
                j += 1
            i = j + 1
            continue
        total += cwidth(s[i])
        i += 1
    return total


def pad_to(s: str, col: int) -> str:
    """Pad with spaces so the next glyph starts at 0-based column `col`."""
    return s + " " * max(0, col - swidth(s))


def repeat_to_width(unit: str, width: int) -> str:
    out, w, uw = "", 0, swidth(unit)
    while w + uw <= width:
        out += unit
        w += uw
    return out


# ---------------------------------------------------------------- block A

def block_a(cols: int) -> list:
    L = []
    L.append("01 ascii  the quick brown fox jumps over the lazy dog" + " " * 8)

    head = "02 cjk-fit "
    L.append(head + repeat_to_width("你好世界", cols - swidth(head)))

    head = "03 cjk-odd "                      # 11 cols → odd offset
    L.append(head + repeat_to_width("汉字宽度", cols - swidth(head) + 12))

    L.append("04 inv-mid abc" + INV + " " + OFF + "def")
    L.append("05 inv-cjk 你好" + INV + " " + OFF + "世界")
    L.append(pad_to("06 inv-eol", cols - 1) + INV + " " + OFF)
    L.append("07 inv-wide " + INV + "汉字" + OFF + " tail")
    L.append("08 bce " + ESC + "[41m" + "red" + ESC + "[K" + OFF)

    # ╭ ─ │ ╰ are EAST ASIAN AMBIGUOUS: a client that calls them wide slides
    # every following cell one column right.
    L.append("09 box " + "╭" + "─" * 12 + "╮")
    L.append("10 box " + "│ 你好, world │")
    L.append("11 box " + "╰" + "─" * 12 + "╯")
    L.append("12 glyph ⏺ ✻ ✽ ⎿ → … ✓ ✗ ● ○ ▪ ⧉ │")
    L.append("13 emoji 🎉 🚀 ✅ 🔥 ⚠️ end")
    L.append("14 para 我需要你帮我在这个 Swift 项目里定位一个渲染 bug，"
             "终端里会出现光标大小的白块 white blocks，"
             "只在切换 tmux pane 之后才出现，initial paint is clean。")
    L.append("15 tail 结尾空格" + " " * 10)
    return L


# ---------------------------------------------------------------- block B
#
# Each entry returns (pane1_row, pane2_row). They are the SAME LENGTH and
# share a prefix, so mosh's frame differ emits a short in-place patch rather
# than a fresh line — the minimal-diff path this bug is reported to live on.

def block_b(cols: int) -> tuple:
    a, b = [], []

    def pair(p1, p2):
        a.append(p1)
        b.append(p2)

    # 20 — a narrow char overwrites a wide char's FIRST half.
    pair("20 half1 前缀你好世界结尾 ABC", "20 half1 前缀X好世界结尾 ABC")
    # 21 — a narrow char overwrites a wide char's SECOND half.
    pair("21 half2 前缀你好世界结尾 ABC", "21 half2 前缀你X世界结尾 ABC")
    # 22 — a wide char lands on top of two narrow ones.
    pair("22 widen 前缀ab好世界结尾 ABC", "22 widen 前缀你好世界结尾 ABC")
    # 23 — an inverse run SHRINKS: 20 cells must lose their reverse bit.
    pair("23 shrink " + INV + " " * 30 + OFF + "|end",
         "23 shrink " + INV + " " * 10 + OFF + " " * 20 + "|end")
    # 24 — …and grows.
    pair("24 grow   " + INV + " " * 10 + OFF + " " * 20 + "|end",
         "24 grow   " + INV + " " * 30 + OFF + "|end")
    # 25 — inverse tail to EOL vs plain tail. If mosh clears the tail with
    #      EL while the reverse rendition is live, a client that erases with
    #      the CURRENT attributes paints a row of white blocks.
    pair(pad_to("25 invtail", 12) + INV + " " * (cols - 12) + OFF,
         pad_to("25 invtail", 12) + "plain tail, nothing set")
    # 26 — the same shape with a background colour instead of reverse.
    pair(pad_to("26 bgtail", 12) + ESC + "[44m" + " " * (cols - 12) + OFF,
         pad_to("26 bgtail", 12) + "plain tail, nothing set")
    # 27 — wrap parity flips: pane 1 ends on a wide char that straddles the
    #      last column, pane 2 is one cell narrower so it fits.
    head = "27 parity "
    pair(head + repeat_to_width("宽字", cols - swidth(head) - 1) + "宽",
         head + "x" + repeat_to_width("宽字", cols - swidth(head) - 2))
    # 28 — claude's block caret steps one column right, over a wide char.
    pair("28 caret 命令行" + INV + " " + OFF + "提示符",
         "28 caret 命令行 " + INV + "提" + OFF + "示符")
    # 29 — a CJK row differing only in its LAST wide char.
    head = "29 lastch "
    body = repeat_to_width("内容", cols - swidth(head) - 2)
    pair(head + body + "甲", head + body + "乙")

    # ---- orphan sweep -------------------------------------------------
    # The mechanism the 90x40 run first caught, swept across end columns.
    #
    # mosh's framebuffer stores a wide char in ONE cell and leaves the next
    # cell an ordinary blank; SwiftTerm stores a width-0 CONTINUATION bound to
    # the char before it. When a repaint makes the new row END one column
    # earlier, mosh sees blank→blank at that column and emits NOTHING — so
    # SwiftTerm keeps a continuation cell with no wide char in front of it,
    # forever, carrying whatever attribute the old wide char had.
    #
    # S — new row is ONE cell shorter than the old wide char's span (orphan).
    # T — new row covers both halves with narrow chars (control: no orphan).
    for k in range(1, 9):
        start = cols - k - 2
        pair(pad_to("S%02d orphan " % k, start) + INV + "汉" + OFF,
             pad_to("S%02d orphan " % k, start) + "a")
    for k in range(1, 9):
        start = cols - k - 2
        pair(pad_to("T%02d narrow " % k, start) + INV + "汉" + OFF,
             pad_to("T%02d narrow " % k, start) + "ab")
    return a, b


def wrapped_rows(line: str, cols: int) -> int:
    """How many SCREEN rows this logical line occupies, honouring the fact
    that a wide char which does not fit leaves a pad cell and wraps."""
    used, col = 1, 0
    i = 0
    while i < len(line):
        if line[i] == ESC:
            j = i + 2
            while j < len(line) and not ("@" <= line[j] <= "~"):
                j += 1
            i = j + 1
            continue
        w = cwidth(line[i])
        if col + w > cols:
            used += 1
            col = 0
        col += w
        i += 1
    return used


def pad_block(lines: list, cols: int, target: int) -> list:
    """Blank-pad a header block so it occupies exactly `target` SCREEN rows.

    Both panes' block-B rows have to land on the SAME screen line — a minimal
    in-place diff is the whole point of block B, and mosh only emits one when
    the old and new rows line up."""
    # Narrow grids wrap block A far enough that it no longer fits above block
    # B. Block B is the repro; block A is the sanity check — so drop block A
    # rows from the end until it fits rather than refusing to run.
    kept = list(lines)
    while sum(wrapped_rows(l, cols) for l in kept) > target and kept:
        kept.pop()
    used = sum(wrapped_rows(l, cols) for l in kept)
    if len(kept) != len(lines):
        import sys
        print(f"note: dropped {len(lines) - len(kept)} header rows to fit {target} rows at {cols} cols",
              file=sys.stderr)
    return kept + [""] * (target - used)


def compose(cols: int, rows: int, which: int) -> str:
    b1, b2 = block_b(cols)
    # Block B has to occupy the same screen rows in both panes; give the
    # header whatever is left over, minus one blank row at the bottom.
    header_rows = rows - len(b1) - 1
    if header_rows < 1:
        raise SystemExit(f"{rows} rows is not enough for {len(b1)} block-B rows")
    if which == 1:
        header = block_a(cols)
        body = b1
    else:
        # A different header so the TOP of the screen is a fresh paint while
        # the BOTTOM is a minimal in-place diff — both mosh code paths land in
        # one switch.
        header = ["P2-%02d 第二个窗格 %s" % (i, repeat_to_width("測試", 20))
                  for i in range(1, min(16, header_rows + 1))]
        body = b2
    for row in body:
        if swidth(row) > cols:
            raise SystemExit(f"block-B row wider than the screen: {row!r}")
    return "\r\n".join(pad_block(header, cols, header_rows) + body) + "\r\n"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--cols", type=int, required=True)
    ap.add_argument("--rows", type=int, default=40)
    ap.add_argument("--pane1", required=True)
    ap.add_argument("--pane2", required=True)
    args = ap.parse_args()
    with open(args.pane1, "w", encoding="utf-8") as f:
        f.write(compose(args.cols, args.rows, 1))
    with open(args.pane2, "w", encoding="utf-8") as f:
        f.write(compose(args.cols, args.rows, 2))


if __name__ == "__main__":
    main()
