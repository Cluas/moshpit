import os, pty, subprocess, time
SOCK="moshiverify"
def tx(*a):
    return subprocess.run(["tmux","-L",SOCK,*a], capture_output=True, text=True)
def disp(fmt):
    return tx("display-message","-p","-t","s1",fmt).stdout.strip()

tx("kill-server")
tx("new-session","-d","-s","s1","-x","80","-y","24")
tx("send-keys","-t","s1",'for i in $(seq 1 300); do echo "L$i hello 你好世界 ┌─┐│x│"; done',"Enter")
time.sleep(0.9)

# Attach a real client on a pty == the mosh-rendered `tmux attach` client.
pid, fd = pty.fork()
if pid == 0:
    os.environ["TERM"]="xterm-256color"
    os.execvp("tmux", ["tmux","-L",SOCK,"attach","-t","s1"])
    os._exit(1)
time.sleep(0.7)
print("attached     :", disp("in_mode=#{pane_in_mode} scroll=#{scroll_position}"))

# (A) sidecar-style server-side scroll (what we used to do) — does it move THIS client?
tx("copy-mode","-t","s1"); tx("send-keys","-t","s1","-N","8","-X","scroll-up")
time.sleep(0.4)
print("server-side  :", disp("in_mode=#{pane_in_mode} scroll=#{scroll_position}"))
tx("send-keys","-t","s1","-X","cancel"); time.sleep(0.3)

# (B) THE FIX: send copy-mode keys to the CLIENT's input (== mosh transport)
os.write(fd, b"\x02["); time.sleep(0.4)            # C-b [  -> enter copy-mode
print("prefix+[     :", disp("in_mode=#{pane_in_mode} scroll=#{scroll_position}"))
os.write(fd, b"\x1b[5~"*3); time.sleep(0.5)        # PageUp x3
print("client PageUp:", disp("in_mode=#{pane_in_mode} scroll=#{scroll_position}"))
print("visible top  :", tx("capture-pane","-p","-t","s1").stdout.splitlines()[0] if tx("capture-pane","-p","-t","s1").stdout else "(empty)")
os.write(fd, b"q"); time.sleep(0.3)                # q -> exit
print("after q      :", disp("in_mode=#{pane_in_mode} scroll=#{scroll_position}"))

try: os.close(fd)
except OSError: pass
tx("kill-server")
print("(isolated socket killed)")
