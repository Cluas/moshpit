#!/usr/bin/env python3
# T1b — prove the SGR wheel bytes our code emits scroll a REAL mouse app over a
# real tmux client, WITHOUT entering copy-mode (so typing keeps working).
#
# Pairs with MoshScrollKeysTests.wheel* (which pins the exact bytes) and with
# verify-tmux-scroll.py (the prefix+[/PageUp copy-mode proof for the shell).
# Together: code emits X (unit test) ∧ X scrolls a real mouse app, no copy-mode
# (this script) ⇒ the wheel path works end-to-end without a device.
#
# The bytes here are exactly SessionHub.ActiveSession.wheelBytes(): SGR mouse,
# button 64 = wheel-up, 65 = wheel-down, at a 1-based cell.
import os, pty, subprocess, sys, time

SOCK = "moshiwheelverify"

def tx(*a):
    return subprocess.run(["tmux", "-L", SOCK, *a], capture_output=True, text=True)

def disp(fmt):
    return tx("display-message", "-p", "-t", "s1", fmt).stdout.strip()

def top_line():
    out = tx("capture-pane", "-p", "-t", "s1").stdout.splitlines()
    return out[0] if out else ""

def wheel(button, col=40, row=12):
    # SGR: ESC [ < button ; col ; row M   (== wheelBytes for one notch)
    return f"\x1b[<{button};{col};{row}M".encode()

tx("kill-server")
tx("new-session", "-d", "-s", "s1", "-x", "80", "-y", "24")
tx("set-option", "-g", "mouse", "on")
# A real alternate-screen mouse app (Claude Code stand-in): less --mouse enables
# DECSET 1000/1006 + the alternate screen and scrolls itself on wheel events.
tx("send-keys", "-t", "s1", "seq 1 300 | less --mouse", "Enter")

# Wait until less is actually up and has requested the mouse.
app_up = False
for _ in range(40):
    if disp("#{mouse_any_flag}") == "1" and disp("#{alternate_on}") == "1":
        app_up = True
        break
    time.sleep(0.1)

# Attach a real client on a pty == the mosh-rendered `tmux attach` client.
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"] = "xterm-256color"
    os.execvp("tmux", ["tmux", "-L", SOCK, "attach", "-t", "s1"])
    os._exit(1)
time.sleep(0.9)

before = top_line()
for _ in range(5):
    os.write(fd, wheel(65))   # wheel-down advances the pager
    time.sleep(0.12)
time.sleep(0.4)
in_mode_after_wheel = disp("#{pane_in_mode}")
after = top_line()
scrolled = before != after

# Typing must still reach the app (no copy-mode swallowed it). 'q' quits less,
# returning to the shell prompt.
os.write(fd, b"q")
time.sleep(0.6)
cmd_after_q = disp("#{pane_current_command}")
typing_reaches_app = "less" not in cmd_after_q.lower()

try:
    os.close(fd)
except OSError:
    pass
tx("kill-server")

print("================ RESULT (tmux wheel) ================")
print(f"  less --mouse came up (mouse_any_flag=1)   : {'PASS' if app_up else 'FAIL'}")
print(f"  wheel forwarded to app (NOT copy-mode)     : {'PASS' if in_mode_after_wheel == '0' else f'FAIL (in_mode={in_mode_after_wheel})'}")
print(f"  app scrolled (top line moved: {before!r} -> {after!r}) : {'PASS' if scrolled else 'FAIL'}")
print(f"  typing reaches the app after scroll (q quit less) : {'PASS' if typing_reaches_app else f'FAIL (cmd={cmd_after_q})'}")
print("(isolated socket killed)")

ok = app_up and in_mode_after_wheel == "0" and scrolled and typing_reaches_app
sys.exit(0 if ok else 1)
