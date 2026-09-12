#!/usr/bin/env python3
"""Mini Claude-Code-like TUI for the tmux -CC lab.

Reproduces the rendering contract that matters for Moshpit's -CC renderer:
  - alternate screen + SGR mouse (DECSET 1049, 1002, 1006) — like Claude Code
  - content lines between STATIC border columns; a real TUI's differ never
    repaints cells it believes unchanged, so the borders (and the shared
    line prefixes) are the probe for lost-cell divergence on the client
  - wheel up/down scrolls the viewport by 3 lines; the repaint is a PER-LINE
    DIFF (common prefix/suffix skipped) against the app's own model
  - SIGWINCH: full clear + repaint at the new size (model reset), the way a
    reflowing TUI answers a tmux window resize

'q' exits. Everything is written unbuffered to fd 1.
"""
import fcntl
import os
import re
import select
import signal
import struct
import sys
import termios

ESC = "\x1b"
MOUSE = re.compile(r"\x1b\[<(\d+);(\d+);(\d+)([Mm])")

winch = False


def on_winch(signum, frame):
    global winch
    winch = True


def term_size():
    try:
        h, w = struct.unpack("HHHH", fcntl.ioctl(1, termios.TIOCGWINSZ, b"\0" * 8))[:2]
        return (h or 24), (w or 80)
    except OSError:
        return 24, 80


def out(s):
    os.write(1, s.encode())


class TUI:
    def __init__(self):
        self.offset = 0
        self.total = 500
        self.gen = 0  # bumped by 'x' — simulates streamed content updates
        self.rows, self.cols = term_size()
        self.model = {}  # row (0-based) -> string the app believes is on screen

    def line_for(self, idx):
        """`│ L0042 4242…42 │` — borders and ` L` prefix are static cells."""
        inner = max(0, self.cols - 2)
        body = f" L{idx:04d} " + f"{idx % 100:02d}" * max(0, (inner - 8) // 2 + 1)
        return "│" + body[:inner].ljust(inner) + "│"

    def status_for(self):
        text = f" offset={self.offset:04d} grid={self.cols}x{self.rows} gen={self.gen:03d} "
        # 256-color SGR before the text: makes every status repaint carry a
        # multi-param sequence, so gate cuts land mid-sequence realistically.
        return ("\x1b[7m\x1b[38;5;210m" + text[: self.cols].ljust(self.cols)
                + "\x1b[27m\x1b[39m", text)

    def content_rows(self):
        return self.rows - 1  # last row = status line

    def paint_full(self):
        self.rows, self.cols = term_size()
        self.model = {}
        out(f"{ESC}[2J{ESC}[H")
        self.paint_diff()

    def paint_diff(self):
        """Repaint only what changed vs the model — a real differ's contract."""
        buf = []
        for r in range(self.content_rows()):
            new = self.line_for(self.offset + r)
            old = self.model.get(r)
            if old == new:
                continue
            if old is None or len(old) != len(new):
                buf.append(f"{ESC}[{r + 1};1H{ESC}[K" + new)
            else:
                # skip common prefix/suffix — static border cells never resent
                a = 0
                while a < len(new) and old[a] == new[a]:
                    a += 1
                b = len(new)
                while b > a and old[b - 1] == new[b - 1]:
                    b -= 1
                buf.append(f"{ESC}[{r + 1};{a + 1}H" + new[a:b])
            self.model[r] = new
        styled, plain = self.status_for()
        if self.model.get(self.content_rows()) != plain:
            buf.append(f"{ESC}[{self.rows};1H{ESC}[K" + styled)
            self.model[self.content_rows()] = plain
        if buf:
            out("".join(buf))

    def scroll(self, lines):
        new_offset = max(0, min(self.total - self.content_rows(),
                                self.offset + lines))
        if new_offset != self.offset:
            self.offset = new_offset
            self.paint_diff()


def main():
    global winch
    signal.signal(signal.SIGWINCH, on_winch)
    attrs = termios.tcgetattr(0)
    new = termios.tcgetattr(0)
    new[3] &= ~(termios.ICANON | termios.ECHO)
    new[6][termios.VMIN] = 0
    new[6][termios.VTIME] = 0
    termios.tcsetattr(0, termios.TCSANOW, new)
    out(f"{ESC}[?1049h{ESC}[?25l{ESC}[?1002h{ESC}[?1006h")
    tui = TUI()
    tui.paint_full()
    try:
        while True:
            if winch:
                winch = False
                tui.paint_full()
            r, _, _ = select.select([0], [], [], 0.05)
            if not r:
                continue
            data = os.read(0, 4096).decode(errors="replace")
            if "q" in data:
                break
            if "x" in data:  # streamed-content tick (desktop typing / agent)
                tui.gen += data.count("x")
                tui.paint_diff()
            for m in MOUSE.finditer(data):
                btn, press = int(m.group(1)), m.group(4) == "M"
                if not press:
                    continue
                if btn == 64:
                    tui.scroll(-3)
                elif btn == 65:
                    tui.scroll(+3)
    finally:
        out(f"{ESC}[?1002l{ESC}[?1006l{ESC}[?25h{ESC}[?1049l")
        termios.tcsetattr(0, termios.TCSANOW, attrs)


if __name__ == "__main__":
    main()
