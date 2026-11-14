import Foundation

/// How Moshpit starts herdr on the remote host.
///
/// Phase 0 runs herdr as a full-screen TUI inside whatever shell the transport
/// already gave us (mosh, or SSH with a PTY) — exactly the shape mosh+tmux has
/// today, with Moshpit acting purely as the renderer. The native control
/// plane (`herdr api snapshot`) and per-pane frame channel
/// (`herdr terminal session control`) come later; see
/// `docs/design/herdr-multiplexer.md`.
enum HerdrLaunch {

    /// Command that attaches the user's herdr session, for writing into an
    /// interactive remote shell (append a `\r` at the call site).
    ///
    /// Two things worth knowing:
    ///
    /// 1. **PATH.** herdr's official installer drops the binary in
    ///    `~/.local/bin` and deliberately does not touch any shell rc file, so
    ///    an interactive login shell often cannot see a herdr the capability
    ///    probe just found. We therefore boot with the same PATH prefix the
    ///    probe searched — otherwise the very command Install Assist hands the
    ///    user yields "herdr: command not found" on the next connect. Same
    ///    trick `MoshBootstrap` uses for `mosh-server`.
    /// 2. **A custom path is trusted verbatim.** It bypasses the probe (the
    ///    user vouched for it), so it must bypass the PATH prefix too.
    ///
    /// Unlike the tmux path, bare `herdr` CREATES a workspace when no server
    /// is running rather than reporting "no sessions". That breaks Moshpit's
    /// "never create a session on the user's behalf" rule, but Phase 0 has no
    /// control plane to detect emptiness with, and the terminal here IS
    /// herdr's own UI — the user sees exactly what typing `herdr` would do.
    /// Phase 1 adds the snapshot probe and the explicit empty state.
    static func attachCommand(customPath: String?) -> String {
        if let customPath, !customPath.isEmpty { return customPath }
        return "PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\" herdr"
    }

    /// A `herdr <subcommand>` invocation for the control channel (see
    /// ``HerdrControlClient``), carrying the same PATH treatment as
    /// ``attachCommand(customPath:)``.
    static func command(_ subcommand: String, customPath: String?) -> String {
        "\(attachCommand(customPath: customPath)) \(subcommand)"
    }

    /// Start a headless herdr server on the host, detached.
    ///
    /// This is the cold-host bootstrap. Moshpit only ever runs it from an
    /// explicit "create a session" tap — the same rule the tmux path follows,
    /// where a server with no sessions gets an empty state rather than one
    /// Moshpit quietly created. (Phase 0 booted herdr's TUI, which started a
    /// server as a side effect; native rendering has no such accident, which
    /// makes the rule easier to honor rather than harder.)
    ///
    /// Starting the server is only half the job: one with no persisted state
    /// comes up with zero workspaces, so the caller has to create one (see
    /// `HerdrControlClient.bootstrapServer`).
    ///
    /// All three descriptors are redirected and `nohup` applied because this
    /// runs on a one-shot exec channel: an inherited stdout would hold the
    /// channel open for the server's whole life, and the channel closing would
    /// otherwise HUP the server we just started.
    static func daemonCommand(customPath: String?) -> String {
        if let customPath, !customPath.isEmpty {
            return "nohup \(customPath) server >/dev/null 2>&1 </dev/null &"
        }
        return "PATH=\"$PATH:\(HostCapabilities.extraPathDirs)\" "
            + "nohup herdr server >/dev/null 2>&1 </dev/null &"
    }

    // MARK: mosh raw-attach renderer (docs/design/roaming-transport.md)

    /// `herdr terminal attach <terminal_id>` renders ONE pane's
    /// raw stream straight to the tty — no sidebar, no header, and the pane
    /// PTY sized to THIS client alone (verified: `stty size` inside reports
    /// the phone grid; the desktop layout is untouched). It is the herdr
    /// analogue of the mosh+tmux renderer's zoomed attach, and strictly
    /// better than driving the shared TUI: nothing about the desktop user's
    /// view changes. Requires a herdr with the `terminal attach` subcommand
    /// (0.8.0 has it) — ``rawAttachProbeCommand`` decides, and hosts without
    /// it keep the full-TUI + immersive-zoom fallback.
    ///
    /// The renderer is a LOOP around the attach: the target lives in a file,
    /// and switching panes means writing the file and bouncing the current
    /// attach (``retargetCommand``) — the loop re-reads and re-attaches.
    /// The loop also self-heals an eviction (another client's --takeover):
    /// the attach exits, the loop attaches again a beat later.

    /// State-file key: connection id ++ a per-BOOT nonce. The nonce is the
    /// generation fence — a loop only ever watches its own generation's
    /// target file, and retargets only write the live generation's, so an
    /// orphaned loop (mosh-server survives disconnects by design) can idle
    /// on its stale file forever without ever re-entering the fight.
    static func moshRendererKey(connectionId: UUID, nonce: String) -> String {
        "\(connectionId.uuidString)-\(nonce)"
    }

    static func moshTargetPath(rendererKey: String) -> String {
        "$HOME/.moshpit/mosh-\(rendererKey).target"
    }

    static func moshPidPath(rendererKey: String) -> String {
        "$HOME/.moshpit/mosh-\(rendererKey).pid"
    }

    /// The loop's own pid: whether the loop still runs at all.
    static func moshLoopPath(rendererKey: String) -> String {
        "$HOME/.moshpit/mosh-\(rendererKey).loop"
    }

    /// Written by the loop when its attach ended on its own and it parked:
    /// `<status> <terminal id>`.
    static func moshStopPath(rendererKey: String) -> String {
        "$HOME/.moshpit/mosh-\(rendererKey).stop"
    }

    /// The attach's stderr — herdr's last words when it ends on its own
    /// ("terminal attach taken over", "… not found", "server is shutting
    /// down").
    static func moshErrPath(rendererKey: String) -> String {
        "$HOME/.moshpit/mosh-\(rendererKey).err"
    }

    /// Created by every retarget: what wakes a parked loop.
    static func moshGoPath(rendererKey: String) -> String {
        "$HOME/.moshpit/mosh-\(rendererKey).go"
    }

    /// The terminal the person chose (a tap, a reply for it, "Open here"),
    /// written by the retarget that points the loop there: the one attach
    /// that may take its pane from another client (`--takeover`). A retarget
    /// that follows the shared focus removes it, and its attach is a plain
    /// one, which herdr refuses while another client holds the terminal
    /// ("… already has an attached client; retry with --takeover", herdr
    /// 0.9.5): following the focus never takes a pane from the device that
    /// has it.
    static func moshTakePath(rendererKey: String) -> String {
        "$HOME/.moshpit/mosh-\(rendererKey).take"
    }

    /// The renderer loop, typed into the mosh shell (append `\r` at the call
    /// site). Single line; waits for the sidecar to publish the first target.
    ///
    /// The attach MUST run in the foreground: backgrounding it (`… & wait`)
    /// leaves the loop shell as the tty's foreground process, and every
    /// keystroke lands there instead of in the pane — output kept flowing
    /// while typing died (found exactly that way, on-device shape). The pid
    /// the retarget needs is captured by a wrapper that writes its own `$$`
    /// and then `exec`s into the attach: same pid, still foreground.
    static func rawAttachLoopCommand(rendererKey: String, customPath: String?) -> String {
        let target = moshTargetPath(rendererKey: rendererKey)
        let pid = moshPidPath(rendererKey: rendererKey)
        let loop = moshLoopPath(rendererKey: rendererKey)
        let stop = moshStopPath(rendererKey: rendererKey)
        let err = moshErrPath(rendererKey: rendererKey)
        let go = moshGoPath(rendererKey: rendererKey)
        // Inner wrapper body, double-quoted for the OUTER shell: $HOME/… in
        // the pid path expands out there; `\$\$` and `\$0` survive into the
        // wrapper. `$0` carries the terminal id so it never re-enters shell
        // parsing. exec drops the wrapper; PATH rides an `export` because an
        // assignment prefix does not survive `exec`. The attach's stderr —
        // herdr's last words when it ends on its own — goes to a file the
        // sidecar reads (`rawAttachWatchCommand`); its screen is stdout.
        // `--takeover` only for the terminal the take file names — the one
        // the person chose (``moshTakePath(rendererKey:)``). Everything else
        // attaches plainly and is refused while another client holds the
        // terminal; the loop parks on that as on any attach that ended on
        // its own.
        let take = moshTakePath(rendererKey: rendererKey)
        let takeover = "k=; [ \\\"\\$(cat \\\"\(take)\\\" 2>/dev/null)\\\" = \\\"\\$0\\\" ] && k=--takeover; "
        let launch: String
        if let customPath, !customPath.isEmpty {
            launch = "\(takeover)exec \(customPath) terminal attach \\\"\\$0\\\" \\$k 2>\\\"\(err)\\\""
        } else {
            launch = "export PATH=\\\"$PATH:\(HostCapabilities.extraPathDirs)\\\"; \(takeover)"
                + "exec herdr terminal attach \\\"\\$0\\\" \\$k 2>\\\"\(err)\\\""
        }
        // Exit-status discrimination is what stops takeover storms: a
        // retarget kills the attach (signal death, $? ≥ 128) and the loop
        // re-attaches; an attach that exits ON ITS OWN — evicted by another
        // client's --takeover, server stopped, pane gone — never grabs the
        // pane back. mosh-server survives app disconnects by design (that's
        // roaming), so every reconnect used to leave an orphaned loop
        // fistfighting the live one over the same target file, 0.3s per
        // round ("页面一直在跳", report, build 360).
        //
        // Such an attach used to BREAK the loop, and nothing ever started
        // it again: the phone sat on the bare login shell under mosh, and
        // whatever was typed or dictated next ran there as a command line —
        // after another device took the pane, the pane closed, or herdr
        // restarted (the app's own Restart button included). Now the loop
        // PARKS: it writes why (`<status> <terminal id>` to the stop file;
        // herdr's words are in the err file), blanks the screen, and waits —
        // the loop shell stays the tty's foreground process, so nothing
        // typed meanwhile reaches a shell — until a retarget creates the go
        // file. The app decides (`rawAttachWatchCommand`): another device
        // has the pane → it says so and waits for the person's one-tap take
        // back; the pane closed or herdr restarted → it points the loop at
        // the focused pane by itself. An orphan's go file is never written
        // (its generation's files are nobody's), so an orphan that loses
        // once still stays down — and goes when its target is removed
        // (`staleRendererCleanupCommand`). Leaving the park, whatever was
        // typed into the tty while it lasted is discarded before the next
        // attach (raw, non-blocking read: `stty -icanon min 0 time 0`),
        // as at the first attach: it was typed at no pane, and the next
        // attach — perhaps another pane's — would read it as its own.
        // The pause before re-attaching is the switch's visible cost — it is
        // spent AFTER our own `kill -9` and before the new attach paints, with
        // the previous pane still on screen the whole time. 0.3s was a round
        // number from when orphaned loops fought over the target file; the
        // nonce fence (see `moshRendererKey`) is what actually settled that,
        // so this is now pure latency. Not zero, though: the loop only ever
        // reaches it after a SIGNAL death, and a signal we didn't send (a
        // supervisor, an OOM killer) would otherwise respawn `herdr` as fast
        // as it can die.
        // Under `sh -c` (B138): typed into a fish login shell, the loop was a
        // syntax error (no `while … do … done`, `$(…)` before 3.4).
        //
        // The wrapper reads the target again once its pid is recorded, and
        // goes round again (137, as if killed) when it has moved on (B208).
        // A retarget's kill reads the pid file, which names the attach being
        // left until the next wrapper writes its own: a switch that landed in
        // between — the loop had read the old target, the new wrapper hadn't
        // written its pid yet — killed a pid already dead, and the loop then
        // attached the pane being LEFT while the app took the new one for on
        // screen. A second switch 50–70 ms behind the first did that live in
        // up to half the tries, and a reply for one agent went to the other,
        // said sent. Now any target written after the wrapper's pid is in the
        // file reaches it by the kill, and any written before by this second
        // read. The go file is consumed only past that read, by the attach it
        // is for: one created later comes with a kill, so a parked loop never
        // finds a stale one and takes a pane back on its own. And the pid file
        // goes with its attach: between attaches, and once the loop has
        // parked, there is no pid to kill — a retarget asked again and again
        // of a loop that is down (B208) never kills a pid the system has since
        // handed to something else.
        //
        // Between two attaches the loop prints nothing of its own but a clear
        // screen. The switch veil lifts once the screen has been quiet a
        // moment, and bash — macOS's sh, and Linux's where sh is bash —
        // reported every killed attach ("Killed: 9") ~10 ms after the kill,
        // while herdr repaints ~250 ms later: the veil lifted on the pane
        // being left for the difference. Its stderr goes nowhere now (the
        // attach's own goes to the err file), and the pane being left is
        // wiped the moment its attach ends: whenever the veil lifts, it never
        // lifts onto the previous pane.
        return Shell.portable("mkdir -p \"$HOME/.moshpit\"; exec 2>/dev/null; echo $$ > \"\(loop)\"; f=1; while :; do "
            + "tid=$(cat \"\(target)\" 2>/dev/null); "
            + "if [ -n \"$tid\" ]; then "
            + "if [ -n \"$f\" ]; then f=; t=$(stty -g 2>/dev/null) "
            + "&& stty -icanon min 0 time 0 2>/dev/null && cat >/dev/null; "
            + "[ -n \"$t\" ] && stty \"$t\" 2>/dev/null; fi; "
            + "sh -c \"echo \\$\\$ > \\\"\(pid)\\\"; "
            + "[ \\\"\\$(cat \\\"\(target)\\\" 2>/dev/null)\\\" = \\\"\\$0\\\" ] || exit 137; "
            + "rm -f \\\"\(go)\\\"; "
            + "\(launch)\" \"$tid\"; "
            + "s=$?; rm -f \"\(pid)\"; printf \"\\033[0m\\033[H\\033[2J\"; "
            + "if [ $s -lt 128 ]; then echo \"$s $tid\" > \"\(stop)\"; "
            + "while [ ! -e \"\(go)\" ]; do [ -e \"\(target)\" ] || exit 0; sleep 0.1; done; "
            + "rm -f \"\(stop)\"; f=1; fi; "
            + "else sleep 0.5; fi; sleep 0.05; done")
    }

    /// Pre-boot cleanup, run on the bootstrap SSH before the new renderer
    /// starts: for every PREVIOUS generation of this connection, remove the
    /// state files and kill any attach it still holds.
    ///
    /// Covers BOTH file layouts. Builds 360/361 used nonce-less paths
    /// (`mosh-<id>.target`) and their loops never learned to lose — they are
    /// IMMORTAL fighters that re-attach 0.3s after any eviction, which kept
    /// "terminal attach taken over" alive on hosts that ever ran those
    /// builds, and stole the screen back from the current session's retarget
    /// (picked a claude-less tab, saw the claude pane — report, build 364).
    /// Order is load-bearing: targets are removed FIRST, then the attach is
    /// killed — the orphan loop wakes from the kill, re-reads a now-missing
    /// target, and idles forever. Kill -9 because a TERM-graceful attach
    /// exit gives the old loop a head start on re-reading before the rm.
    ///
    /// Only a pid whose command line is that generation's renderer is
    /// killed: `terminal attach`, then its target's terminal id (as the
    /// attach runs it, `herdr terminal attach <id> --takeover`, and as its
    /// wrapper does, the id last). A pid file outlives its loop when the loop
    /// dies with its mosh-server (stopped, or the host rebooted), and the
    /// system hands the pid to something else: the next connect used to
    /// `kill -9` whatever had it now. No target, no number, or a command line
    /// that isn't the renderer's: left alone. Linux hosts are read from
    /// `/proc/<pid>/cmdline`, others (macOS) from `ps -o args=` — both in
    /// any login shell, under `sh -c` (B138).
    static func staleRendererCleanupCommand(connectionId: UUID) -> String {
        let base = "$HOME/.moshpit/mosh-\(connectionId.uuidString)"
        return "for f in \"\(base)\".pid \"\(base)-\"*.pid; do "
            + "[ -f \"$f\" ] || continue; k=${f%.pid}; "
            + "t=$(cat \"$k.target\" 2>/dev/null); rm -f \"$k.target\"; "
            + "p=$(cat \"$f\" 2>/dev/null); "
            + "case \"$p\" in ''|*[!0-9]*) continue ;; esac; "
            + "[ -n \"$t\" ] || continue; "
            + commandLineOf("$p")
            + "case \"$a\" in *\"terminal attach\"*\"$t\"*) kill -9 \"$p\" 2>/dev/null ;; esac; done; "
            + "rm -f \"\(base)\".target \"\(base)-\"*.target \"\(base)\".pid \"\(base)-\"*.pid "
            + "\"\(base)-\"*.go \"\(base)-\"*.stop \"\(base)-\"*.err \"\(base)-\"*.loop \"\(base)-\"*.take; true"
    }

    /// Sets `a` to the command line of process `pid` (a shell word: `$p`),
    /// or empty: `/proc` where there is one (Linux, without forking `ps`),
    /// `ps -o args=` otherwise (macOS, BSD).
    static func commandLineOf(_ pid: String) -> String {
        "a=; [ -r \"/proc/\(pid)/cmdline\" ] && a=$(tr \"\\000\" \" \" < \"/proc/\(pid)/cmdline\" 2>/dev/null); "
            + "[ -n \"$a\" ] || a=$(ps -o args= -p \"\(pid)\" 2>/dev/null); "
    }

    /// Point the loop at a new terminal: publish the target, bounce the
    /// running attach, and say so once the loop has come round to it
    /// (``retargetLanded(_:)``). Runs on the sidecar's exec channel.
    /// Idempotence is the caller's job (skip when the target hasn't changed)
    /// — the bounce itself costs a visible reattach flicker.
    ///
    /// `takeover`: the person chose this pane (a tap, a reply for it, "Open
    /// here"), and its attach takes it from whichever client holds it
    /// (``moshTakePath(rendererKey:)``). Otherwise the retarget follows the
    /// shared focus and its attach is a plain one: following the focus
    /// always took over, so a second device lost its pane whenever the other
    /// followed the focus onto it. herdr refuses a plain attach to a held
    /// terminal ~260 ms after it starts ("… already has an attached client;
    /// retry with --takeover", herdr 0.9.5, measured) and the loop parks on
    /// it. A plain attach is watched that long past its start before the
    /// switch counts: one the loop parked on is said here as the watch says
    /// it (``rendererStop(_:)``: ``rendererStoppedMarker`` and the stop
    /// record), so nothing typed for the pane goes into an attach about to
    /// be refused. The take file is written before the target: a wrapper
    /// that reads the new target reads the take file that goes with it.
    ///
    /// Writing the file and killing the attach used to be all, and the exec
    /// coming back was taken for the pane being on screen (B208). It isn't
    /// proof of anything: a loop that was gone — it used to end when its
    /// attach did (another client's `--takeover`, its pane closed, its
    /// server stopped), and now ends only with its shell — leaves a bare
    /// login shell under mosh, and the write and the kill succeed all the
    /// same, so a reply sent "to the pane" was typed into that shell as a
    /// command line. A parked loop is woken by the go file the retarget
    /// creates; a gone one is said (``rendererGoneMarker``). The exec waits,
    /// at most 2 s, for the loop to start its next attach (a pid in the file
    /// other than the one killed, alive) and prints the marker only then;
    /// with the loop's own second read of the target
    /// (``rawAttachLoopCommand(rendererKey:customPath:)``) that attach is
    /// this target's. Input sent from then on waits in the tty for
    /// it: herdr's attach reads what was typed before it came up — measured
    /// live over mosh, nothing lost sent 0 to 300 ms behind the kill.
    ///
    /// A terminal the server no longer lists — its pane closed since the
    /// app's last read, a reply for an agent that is gone — is refused
    /// before the loop is touched: ``retargetMissingMarker``, the attach on
    /// screen left as it is. It used to be written and the attach killed,
    /// and the marker came as soon as the next wrapper had its pid, ~350 ms
    /// before herdr refused that attach ("terminal … not found"): the switch
    /// counted, what was typed for the pane went to an attach on its way
    /// out, and the renderer was down. Asked of the server with `customPath`,
    /// the herdr the loop runs; a server that doesn't answer (not running,
    /// another protocol) is no answer, and the switch goes ahead as before.
    static func retargetCommand(terminalId: String, rendererKey: String, customPath: String?,
                                takeover: Bool) -> String {
        let target = moshTargetPath(rendererKey: rendererKey)
        let pid = moshPidPath(rendererKey: rendererKey)
        // -9 is load-bearing: herdr handles SIGTERM gracefully and exits
        // BELOW 128, which the loop's discrimination reads as "evicted, stay
        // down" — a plain kill would make every retarget the renderer's
        // last. SIGKILL is uncatchable, so the loop sees 137 and re-attaches
        // (verified live both ways).
        let loop = moshLoopPath(rendererKey: rendererKey)
        let go = moshGoPath(rendererKey: rendererKey)
        let take = moshTakePath(rendererKey: rendererKey)
        let stop = moshStopPath(rendererKey: rendererKey)
        let err = moshErrPath(rendererKey: rendererKey)
        let chosen = takeover
            ? "printf '%s' \(quote(terminalId)) > \"\(take)\"; "
            : "rm -f \"\(take)\"; "
        // A plain attach counts once herdr has had its moment to refuse it
        // (12 looks, 50 ms apart): parked by then, the stop record is printed.
        let settle = takeover ? "" : "j=0; while [ $j -lt 12 ]; do if [ -s \"\(stop)\" ]; then "
            + "echo \(rendererStoppedMarker); cat \"\(stop)\" \"\(err)\" 2>/dev/null; exit 0; fi; "
            + "sleep 0.05; j=$((j+1)); done; "
        // The go file wakes a loop that parked (its attach ended on its
        // own); one still attached is woken by the kill.
        return "t=\(quote(terminalId)); "
            + "s=$(\(command("api snapshot", customPath: customPath)) 2>/dev/null); "
            + "case \"$s\" in *'\"result\"'*) case \"$s\" in *\"\\\"$t\\\"\"*) ;; "
            + "*) echo \(retargetMissingMarker); exit 0 ;; esac ;; esac; "
            + chosen
            + "printf '%s' \(quote(terminalId)) > \"\(target)\"; : > \"\(go)\"; "
            + "old=$(cat \"\(pid)\" 2>/dev/null); "
            // Only a renderer: the pid file is the loop's, but one that died
            // with its shell left it behind, its pid free for the system to
            // give to anything.
            + "case \"$old\" in ''|*[!0-9]*) ;; *) " + commandLineOf("$old")
            + "case \"$a\" in *\"terminal attach\"*) kill -9 \"$old\" 2>/dev/null ;; esac ;; esac; "
            + "i=0; while [ $i -lt 40 ]; do "
            + "new=$(cat \"\(pid)\" 2>/dev/null); "
            + "if [ -n \"$new\" ] && [ \"$new\" != \"$old\" ] && kill -0 \"$new\" 2>/dev/null; then "
            + settle
            + "echo \(retargetLandedMarker); break; fi; "
            // No loop left to come round — it was killed, its shell is the
            // tty's again: said at once, not after the 2 s. A loop that
            // never wrote its pid (its boot line eaten by a shell still
            // starting) is said at the end of them.
            + "l=; read l < \"\(loop)\" 2>/dev/null; "
            + "[ -n \"$l\" ] && ! kill -0 \"$l\" 2>/dev/null && { echo \(rendererGoneMarker); break; }; "
            + "sleep 0.05; i=$((i+1)); done; "
            + "[ -s \"\(loop)\" ] || echo \(rendererGoneMarker); true"
    }

    /// Waits, on the sidecar, for the loop to stop showing the attach it is
    /// on now: prints ``rendererStoppedMarker`` and the stop record
    /// (`<status> <terminal id>`, then herdr's last words) when the attach
    /// ended on its own and the loop parked, ``rendererGoneMarker`` when the
    /// loop itself is gone. Nothing when a switch moved it on (a new pid in
    /// the file — the next landing arms the next watch) or after 60 s (300
    /// looks, 200 ms apart) — the caller asks again. Reads only.
    ///
    /// The app arms one after every landing: a takeover by another device,
    /// the pane closing or herdr restarting change nothing the poll sees in
    /// time, and until the app knows, what is typed goes to a loop that
    /// shows no pane.
    static func rawAttachWatchCommand(rendererKey: String) -> String {
        let pid = moshPidPath(rendererKey: rendererKey)
        let loop = moshLoopPath(rendererKey: rendererKey)
        let stop = moshStopPath(rendererKey: rendererKey)
        let err = moshErrPath(rendererKey: rendererKey)
        return "w=; read w < \"\(pid)\" 2>/dev/null; i=0; while [ $i -lt 300 ]; do "
            + "if [ -s \"\(stop)\" ]; then echo \(rendererStoppedMarker); "
            + "cat \"\(stop)\" \"\(err)\" 2>/dev/null; break; fi; "
            + "l=; read l < \"\(loop)\" 2>/dev/null; "
            + "[ -n \"$l\" ] && ! kill -0 \"$l\" 2>/dev/null && { echo \(rendererGoneMarker); break; }; "
            + "p=; read p < \"\(pid)\" 2>/dev/null; "
            + "[ -n \"$p\" ] && [ \"$p\" != \"$w\" ] && break; "
            + "sleep 0.2; i=$((i+1)); done; true"
    }

    /// What ``rawAttachWatchCommand(rendererKey:)`` prints when the loop
    /// parked: its attach ended on its own.
    static let rendererStoppedMarker = "MOSHPIT_RENDERER_STOPPED"

    /// What the watch and the retarget print when the loop itself is gone.
    static let rendererGoneMarker = "MOSHPIT_RENDERER_GONE"

    /// How the loop stopped showing its pane, read from a watch's (or a
    /// retarget's) output; nil when it says neither.
    enum RendererStop: Equatable {
        /// The attach ended on its own and the loop parked: its exit status,
        /// the terminal it was on, and herdr's last words.
        case parked(status: Int, terminal: String, said: String)
        /// The loop itself is gone.
        case loopGone
    }

    /// Read by bytes: the record is the line after the marker, `<status>
    /// <terminal id>` (a status that isn't 1–9 ASCII digits reads 0), and
    /// herdr's words are the rest, ASCII whitespace trimmed.
    static func rendererStop(_ output: Data) -> RendererStop? {
        let bytes = [UInt8](output)
        if let at = firstRange(of: Array(rendererStoppedMarker.utf8), in: bytes) {
            var start = at.upperBound
            while start < bytes.count, bytes[start] == 0x0A || bytes[start] == 0x0D { start += 1 }
            let end = bytes[start...].firstIndex(of: 0x0A) ?? bytes.count
            let record = bytes[start..<end]
            let space = record.firstIndex(of: 0x20)
            let head = record[record.startIndex..<(space ?? record.endIndex)]
            let digits = !head.isEmpty && head.count <= 9 && head.allSatisfy { (0x30...0x39).contains($0) }
            let status = digits ? Int(String(decoding: head, as: UTF8.self)) ?? 0 : 0
            let terminal = space.map { String(decoding: record[($0 + 1)...], as: UTF8.self) } ?? ""
            var said = end < bytes.count ? bytes[(end + 1)...] : []
            let blank: Set<UInt8> = [0x20, 0x09, 0x0A, 0x0D, 0x0B, 0x0C]
            while let first = said.first, blank.contains(first) { said = said.dropFirst() }
            while let last = said.last, blank.contains(last) { said = said.dropLast() }
            return .parked(status: status, terminal: terminal, said: String(decoding: said, as: UTF8.self))
        }
        if firstRange(of: Array(rendererGoneMarker.utf8), in: bytes) != nil { return .loopGone }
        return nil
    }

    private static func firstRange(of needle: [UInt8], in haystack: [UInt8]) -> Range<Int>? {
        guard !needle.isEmpty, haystack.count >= needle.count else { return nil }
        for start in 0...(haystack.count - needle.count)
        where haystack[start..<(start + needle.count)].elementsEqual(needle) {
            return start..<(start + needle.count)
        }
        return nil
    }

    /// Why a parked loop's attach ended, as far as the app acts on it.
    enum RendererStopKind: Equatable {
        /// Another client attached the terminal with `--takeover`, or holds
        /// it and a plain attach (one that followed the focus) was refused:
        /// the pane is on another device. Never taken without the person.
        case takenOver
        /// The pane closed, or the server stopped or restarted (terminal ids
        /// are new after a restart): the renderer goes to the focused pane.
        case gone
        /// Anything else herdr refused with: said, retried on the person's
        /// word only.
        case refused
    }

    /// herdr 0.9.5's last words, measured live: "terminal attach taken
    /// over", "terminal attach failed: terminal … already has an attached
    /// client; retry with --takeover" (a plain attach to a terminal another
    /// client holds), "terminal attach ended: terminal … not found" (its
    /// pane closed), "terminal attach failed: terminal … not found" (no such
    /// terminal), "server is shutting down" — each behind "herdr: server
    /// shut down: ". A shell that couldn't run herdr at all (126, 127) is
    /// refused, whatever it said ("herdr: not found" is no pane closing).
    /// Words this doesn't know fall back on the tree: a terminal the server
    /// no longer lists, or no server, is gone; one it lists was refused.
    static func rendererStopKind(status: Int, said: String,
                                 terminalListed: Bool, serverUp: Bool) -> RendererStopKind {
        if status == 126 || status == 127 { return .refused }
        if said.contains("taken over") || heldElsewhere(said) { return terminalListed ? .takenOver : .gone }
        if said.contains("shutting down") { return .gone }
        if said.contains("terminal attach"), said.contains("not found") { return .gone }
        if !serverUp || !terminalListed { return .gone }
        return .refused
    }


    /// Whether herdr's words refuse a plain attach because another client
    /// holds the terminal (herdr 0.9.5: "terminal attach failed: terminal …
    /// already has an attached client; retry with --takeover"; the frame
    /// channel's `terminal.closed` says the same).
    static func heldElsewhere(_ said: String) -> Bool {
        said.contains("already has an attached client") || said.contains("retry with --takeover")
    }

    /// What ``retargetCommand(terminalId:rendererKey:customPath:takeover:)`` prints
    /// when the server doesn't list the terminal: the loop was not touched.
    static let retargetMissingMarker = "MOSHPIT_RETARGET_MISSING"

    /// Whether a retarget's output says its terminal is gone.
    static func retargetMissing(_ output: Data) -> Bool {
        String(decoding: output, as: UTF8.self).contains(retargetMissingMarker)
    }

    /// What ``retargetCommand(terminalId:rendererKey:customPath:takeover:)`` prints once the
    /// loop has started the target's attach.
    static let retargetLandedMarker = "MOSHPIT_RETARGETED"

    /// Whether a retarget's output says the loop came round to its target —
    /// the only thing that makes the switch the pane on screen.
    static func retargetLanded(_ output: Data) -> Bool {
        String(decoding: output, as: UTF8.self).contains(retargetLandedMarker)
    }

    /// Whether the loop is still on `terminalId`: the target file names it
    /// and the attach whose pid is in the file is alive. For input to the
    /// pane already on screen, which no retarget has just vouched for (B208):
    /// the loop ends under it when another client takes the pane over or the
    /// server restarts, mosh then shows the login shell, and the pid file is
    /// gone with the attach (``rawAttachLoopCommand(rendererKey:customPath:)``).
    /// Prints ``rawAttachUpMarker`` when it is; reads only, kills nothing.
    ///
    /// When it isn't because the loop parked or is gone, it says so as the
    /// watch would (``rendererStop(_:)``): a reply that beat the watch to a
    /// takeover must not read as a switch that never landed — that had the
    /// next poll point the loop back at the pane, taking it from the device
    /// that has it.
    static func rawAttachCheckCommand(terminalId: String, rendererKey: String) -> String {
        let target = moshTargetPath(rendererKey: rendererKey)
        let pid = moshPidPath(rendererKey: rendererKey)
        let loop = moshLoopPath(rendererKey: rendererKey)
        let stop = moshStopPath(rendererKey: rendererKey)
        let err = moshErrPath(rendererKey: rendererKey)
        return "[ \"$(cat \"\(target)\" 2>/dev/null)\" = \(quote(terminalId)) ] && "
            + "kill -0 \"$(cat \"\(pid)\" 2>/dev/null)\" 2>/dev/null && echo \(rawAttachUpMarker); "
            + "if [ -s \"\(stop)\" ]; then echo \(rendererStoppedMarker); cat \"\(stop)\" \"\(err)\" 2>/dev/null; "
            + "else l=; read l < \"\(loop)\" 2>/dev/null; "
            + "[ -n \"$l\" ] && ! kill -0 \"$l\" 2>/dev/null && echo \(rendererGoneMarker); fi; true"
    }

    /// Whether the loop the start line was typed for is running: its pid
    /// file names a live process. Asked before the loop is first given a
    /// target (and after a reboot): a login shell still starting can swallow
    /// a line typed into it — Powerlevel10k's instant prompt flushes pending
    /// input, a startup file reads stdin — and the loop never starts; its
    /// first switch then said the loop was gone after 2 s, and only the
    /// person's Retry typed the line again. Waits up to 1.5 s (30 looks, 50
    /// ms apart) and prints ``loopUpMarker`` once it is up; reads only.
    static func rawAttachLoopUpCommand(rendererKey: String) -> String {
        let loop = moshLoopPath(rendererKey: rendererKey)
        return "i=0; while [ $i -lt 30 ]; do l=; read l < \"\(loop)\" 2>/dev/null; "
            + "[ -n \"$l\" ] && kill -0 \"$l\" 2>/dev/null && { echo \(loopUpMarker); break; }; "
            + "sleep 0.05; i=$((i+1)); done; true"
    }

    /// What ``rawAttachLoopUpCommand(rendererKey:)`` prints once the loop runs.
    static let loopUpMarker = "MOSHPIT_LOOP_UP"

    /// Whether a loop-up check's output says the loop runs.
    static func loopUp(_ output: Data) -> Bool {
        String(decoding: output, as: UTF8.self).contains(loopUpMarker)
    }

    /// What ``rawAttachCheckCommand(terminalId:rendererKey:)`` prints when
    /// the loop is on its terminal.
    static let rawAttachUpMarker = "MOSHPIT_ATTACHED"

    /// Whether a check's output says the loop is on its terminal.
    static func rawAttachUp(_ output: Data) -> Bool {
        String(decoding: output, as: UTF8.self).contains(rawAttachUpMarker)
    }

    /// Does this herdr know `terminal attach`? Prints the marker on yes;
    /// anything else (old herdr, no herdr) prints nothing and the mosh
    /// renderer falls back to the full TUI + immersive zoom.
    static func rawAttachProbeCommand(customPath: String?) -> String {
        "\(attachCommand(customPath: customPath)) terminal attach --help >/dev/null 2>&1 "
            + "&& echo MOSHPIT_RAW_ATTACH_OK"
    }

    /// POSIX single-quote a value destined for the remote shell: the app's one
    /// quoter, ``Shell/quote(_:)``.
    ///
    /// Workspace and tab labels can come from the user or from another client
    /// entirely, pane working directories and agent names from whatever runs
    /// on the host, so they are untrusted: an unescaped `'` would close the
    /// quote and let the rest of the string run as commands.
    static func quote(_ value: String) -> String {
        Shell.quote(value)
    }
}

/// When the veil over a mosh+herdr pane switch comes down.
///
/// mosh ships screen diffs, not frames, so nothing says "this is the new
/// pane" — the signal is the repaint itself: the veil lifts once the bytes
/// stop for ``quiet``. Which bytes count was the bug: any did, and the first
/// to come after the retarget's kill were the loop's own — bash's "Killed: 9"
/// ~10 ms on, herdr's repaint ~250 ms on — or the pane being left still
/// printing: the veil lifted on the previous pane for the difference. Bytes
/// count from the landing now (the host's word that the loop came round,
/// ``HerdrLaunch/retargetLanded(_:)``), and the quiet is counted from the
/// landing too: a repaint that beat the landing's exec back has already
/// painted, one that is late keeps the veil up while it comes. Capped by
/// ``cap``, for a pane that paints nothing at all.
struct MoshSwitchCover: Equatable {
    static let quiet: TimeInterval = 0.15
    static let cap: TimeInterval = 3

    let since: Date
    private(set) var landedAt: Date?
    private(set) var lastPaint: Date?

    init(since: Date) { self.since = since }

    /// Bytes arrived under the veil: they count once the switch has landed.
    mutating func painted(at time: Date) {
        guard landedAt != nil else { return }
        lastPaint = time
    }

    /// The loop came round to the pane: from here the screen is its own.
    mutating func landed(at time: Date) {
        if landedAt == nil { landedAt = time }
    }

    /// Whether the veil comes down at `now`.
    func lifts(at now: Date) -> Bool {
        if now.timeIntervalSince(since) >= Self.cap { return true }
        guard let landedAt else { return false }
        return now.timeIntervalSince(max(landedAt, lastPaint ?? landedAt)) >= Self.quiet
    }
}
