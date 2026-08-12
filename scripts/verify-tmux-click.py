#!/usr/bin/env python3
# Prove the SGR click bytes our tap-to-position code emits reach the PROGRAM
# running in a tmux pane — past tmux's own mouse handling, which under
# `mouse on` would otherwise keep a click for itself (pane focus, selection) and
# leave the program none the wiser.
#
# Pairs with ClickBytesTests (which pins the exact bytes) and with
# verify-tmux-wheel.py (the same proof for the wheel). Together: code emits X
# (unit test) ∧ tmux delivers X to the program (this script) ⇒ any program that
# understands an SGR click — Claude Code's prompt, vim, less — moves its cursor
# to the tapped cell. That last step was also confirmed by hand against a real
# Claude Code in a real tmux pane: clicking column 9 of its input line moved
# tmux's reported #{cursor_x} from 28 to 8.
#
# The pane runs a stand-in that does what such a program does at the protocol
# level: turn mouse reporting on (DECSET 1002/1006, which is what tmux reads as
# `#{mouse_any_flag}`), then echo every byte it receives so the delivered report
# is visible. The bytes sent are exactly
# SessionHub.ActiveSession.clickBytes(col:row:), delivered the way
# TmuxSessionController.sendInput does it: `send-keys -H <hex>`.
import os, pty, subprocess, sys, time

SOCK = "moshiclickverify"
# clickBytes(col: 8, row: 29) — 0-based cell in, 1-based on the wire.
CLICK = b"\x1b[<0;9;30M\x1b[<0;9;30m"


def tx(*a):
    return subprocess.run(["tmux", "-L", SOCK, *a], capture_output=True, text=True)


def disp(fmt):
    return tx("display-message", "-p", "-t", "s1", fmt).stdout.strip()


def pane_text():
    return tx("capture-pane", "-p", "-t", "s1").stdout


tx("kill-server")
tx("new-session", "-d", "-s", "s1", "-x", "80", "-y", "24")
tx("set-option", "-g", "mouse", "on")
# Mouse-aware stand-in: ask for SGR mouse reports (1002 = button events,
# 1006 = SGR encoding), then dump what arrives with control bytes made visible.
tx("send-keys", "-t", "s1",
   r"printf '\033[?1002h\033[?1006h'; stty raw -echo; cat -v", "Enter")

app_up = False
for _ in range(40):
    if disp("#{mouse_any_flag}") == "1":
        app_up = True
        break
    time.sleep(0.1)

# Attach a real client on a pty, so tmux is doing its normal client-attached
# bookkeeping rather than a headless special case.
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execvp("tmux", ["tmux", "-L", SOCK, "attach", "-t", "s1"])
    os._exit(1)
time.sleep(0.9)

before = pane_text()
hexed = " ".join(f"{b:02x}" for b in CLICK)
tx("send-keys", "-t", "s1", "-H", *hexed.split(" "))
time.sleep(0.6)
after = pane_text()
received = after[len(before):] if after.startswith(before) else after

# `cat -v` renders ESC as ^[ — so the report the program got reads back as
# ^[[<0;9;30M ^[[<0;9;30m if, and only if, tmux handed it over intact.
delivered = "^[[<0;9;30M^[[<0;9;30m" in received.replace("\n", "").replace(" ", "")
# A click must not put the pane into copy-mode (that would swallow typing).
in_mode = disp("#{pane_in_mode}")

try:
    os.close(fd)
except OSError:
    pass
tx("kill-server")

print("================ RESULT (tmux click) ================")
print(f"  mouse-aware program up (mouse_any_flag=1)  : {'PASS' if app_up else 'FAIL'}")
print(f"  click delivered to the program intact      : {'PASS' if delivered else f'FAIL (got {received!r})'}")
print(f"  pane NOT left in copy-mode                 : {'PASS' if in_mode == '0' else f'FAIL (in_mode={in_mode})'}")
print("(isolated socket killed)")

ok = app_up and delivered and in_mode == "0"
sys.exit(0 if ok else 1)
