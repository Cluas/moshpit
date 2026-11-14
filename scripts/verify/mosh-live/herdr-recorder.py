# mosh-live herdr-retarget (run.sh herdr): runs in a pane of the private
# herdr, shows a marker, then records every byte typed into the pane (raw
# tty, no echo) to a log, one line per read: <epoch> <hex>. The screen stays
# three lines: name, marker (30 of the letter), status.
#   python3 -I herdr-recorder.py <letter> <log>
import os, sys, time, tty
name, log = sys.argv[1], sys.argv[2]
fd = sys.stdin.fileno()
tty.setraw(fd)
out = os.fdopen(os.dup(1), "w")
out.write("\x1b[2J\x1b[H== PANE %s ==\r\n%s\r\n" % (name, name * 30))
out.flush()
count = 0
with open(log, "a", buffering=1) as f:
    while True:
        data = os.read(fd, 4096)
        if not data:
            break
        f.write("%.6f %s\n" % (time.time(), data.hex()))
        count += len(data)
        out.write("\x1b[3;1H\x1b[2K%s got %d bytes (total %d)" % (name, len(data), count))
        out.flush()
