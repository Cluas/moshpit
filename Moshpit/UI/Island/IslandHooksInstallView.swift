import SwiftUI
import UIKit

/// Install Moshpit's Vibe Island agent hooks on the remote host.
///
/// Phase B "hook stamp" bridge: coding-agent hooks (Claude Code) stamp the
/// current tmux pane with `@moshpit_*` user options so Moshpit reads PRECISE agent
/// status (working / needs you / done) over the existing tmux -CC channel,
/// instead of guessing from output. This sheet mirrors `InstallAssistView`:
///   - Run in terminal — pastes + sends the one-liner installer into the live
///     shell so the user watches it run (it backs up settings.json first).
///   - Copy command — copies the installer to the clipboard.
///   - Re-check — asks the live tmux controller to poll pane state; resolves
///     once ANY pane reports a non-empty `@moshpit_state`.
///
/// When no live session is attached the sheet still shows the command + Copy;
/// Run-in-terminal and Re-check are disabled (the `.noChannel` path).
struct IslandHooksInstallView: View {
    /// The live session, when one is attached. nil → command-only mode.
    let session: SessionHub.ActiveSession?

    @Environment(\.dismiss) private var dismiss
    @State private var rechecking = false
    @State private var recheckResult: RecheckResult?
    @State private var copied = false
    /// Non-nil when Run was refused because the active pane is an agent's
    /// input box — the message names the host and the offending command.
    @State private var runBlockedMessage: String?
    @State private var selectedAgent: IslandAgent = IslandAgent.all[0]

    private enum RecheckResult: Equatable { case resolved, stillMissing, noChannel }

    /// The install one-liner for the currently-selected agent.
    private var command: String { Self.installCommand(for: selectedAgent) }

    /// The active tmux control surface (SSH+tmux or mosh+tmux sidecar), if any.
    private var controller: TmuxSessionController? { session?.tmuxControl }

    /// True when we can paste into a live shell.
    private var hasLiveShell: Bool { session != nil }

    var body: some View {
        VStack(spacing: 0) {
            header
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    intro
                    agentPicker
                    whatItDoes
                    commandBlock
                    if let result = recheckResult { resultRow(result) }
                }
                .padding(.horizontal, Metrics.pageHPad)
                .padding(.top, 14)
                .padding(.bottom, 28)
            }
        }
        .background { MoshpitBackground() }
        .preferredColorScheme(.dark)
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .moshpitCard(isPresented: Binding(
            get: { runBlockedMessage != nil },
            set: { if !$0 { runBlockedMessage = nil } }
        )) {
            MoshpitNoticeCard(icon: "play.slash.fill", tone: .warn,
                              title: "Not run", message: runBlockedMessage ?? "") {
                runBlockedMessage = nil
            }
        }
    }

    // MARK: - Header

    private var header: some View {
        HStack {
            Text("Install agent hooks")
                .font(Face.text(17, .semibold))
                .foregroundStyle(Ink.primary)
            Spacer()
            Button {
                dismiss()
            } label: {
                Text("Done").font(Face.text(15, .semibold)).foregroundStyle(Ink.accent)
            }
            .buttonStyle(.plain)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 14)
        .background(Ink.navGlass)
        .overlay(alignment: .bottom) {
            Rectangle().fill(Ink.hairline).frame(height: 1)
        }
    }

    // MARK: - Body sections

    private var intro: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Precise agent status")
                .font(Face.text(14, .semibold))
                .foregroundStyle(Ink.primary)
            Text("Install Moshpit's hooks so the Vibe Island shows exactly when your agent is working, what it's running, when it needs you, and when it's done — instead of guessing from output. Moshpit backs up your config first and never blocks the agent.")
                .font(Face.text(12))
                .foregroundStyle(Ink.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// Pick which coding agent to install hooks for. The stamp script is shared;
    /// each agent just registers it in its own config (path shown in the footer).
    private var agentPicker: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("AGENT")
                .font(Face.mono(10.5, .semibold))
                .kerning(0.9)
                .foregroundStyle(Ink.meta)
            Menu {
                ForEach(IslandAgent.all) { agent in
                    Button {
                        selectedAgent = agent
                        copied = false
                        recheckResult = nil
                    } label: {
                        if agent.id == selectedAgent.id {
                            Label(agent.displayName, systemImage: "checkmark")
                        } else {
                            Text(agent.displayName)
                        }
                    }
                }
            } label: {
                HStack {
                    Text(selectedAgent.displayName)
                        .font(Face.text(14, .semibold))
                        .foregroundStyle(Ink.primary)
                    Spacer()
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(Ink.accent)
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 11)
                .background(Ink.groupRaised,
                            in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                        .strokeBorder(Ink.cardBorder, lineWidth: 1))
            }
            .accessibilityIdentifier("islandhooks-agent")
        }
    }

    private var whatItDoes: some View {
        VStack(alignment: .leading, spacing: 12) {
            detailRow("checkmark.shield.fill", Ink.success,
                      "Backs up your config",
                      "Copies the agent's config to a timestamped backup before merging Moshpit's hook groups.")
            detailRow("bolt.slash.fill", Ink.accent,
                      "Never blocks the agent",
                      "The hooks only stamp the tmux pane and exit 0 — the agent is never slowed, prompted, or interrupted.")
            detailRow("arrow.triangle.2.circlepath", Ink.mosh,
                      "Safe to re-run",
                      "Idempotent: re-running de-dupes Moshpit's hooks instead of stacking them.")
        }
    }

    private func detailRow(_ icon: String, _ tint: Color,
                           _ title: LocalizedStringKey, _ body: LocalizedStringKey) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: icon)
                .font(.system(size: 15))
                .foregroundStyle(tint)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(Face.text(13, .semibold)).foregroundStyle(Ink.primary)
                Text(body).font(Face.text(12)).foregroundStyle(Ink.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var commandBlock: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(command)
                .font(Face.mono(11))
                .foregroundStyle(Ink.primary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(12)
                .background(Ink.terminalBG,
                            in: RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Metrics.cardRadius, style: .continuous)
                        .strokeBorder(Ink.cardBorder, lineWidth: 1))
                .accessibilityIdentifier("islandhooks-command")

            HStack(spacing: 10) {
                actionButton(
                    title: String(localized: "Run in terminal"),
                    systemImage: "terminal", filled: true,
                    enabled: hasLiveShell
                ) {
                    runInTerminal(command)
                }
                .accessibilityIdentifier("islandhooks-run")

                actionButton(
                    title: copied ? String(localized: "Copied") : String(localized: "Copy command"),
                    systemImage: copied ? "checkmark" : "doc.on.doc", filled: false,
                    enabled: true
                ) {
                    UIPasteboard.general.string = command
                    copied = true
                }
                .accessibilityIdentifier("islandhooks-copy")
            }

            Text("Edits \(selectedAgent.configHint) (a timestamped backup is written first).")
                .font(Face.text(11))
                .foregroundStyle(Ink.meta)
                .fixedSize(horizontal: false, vertical: true)

            recheckButton
        }
    }

    private var recheckButton: some View {
        Button {
            recheck()
        } label: {
            HStack(spacing: 6) {
                if rechecking {
                    ProgressView().tint(Ink.accent).scaleEffect(0.8)
                } else {
                    Image(systemName: "arrow.clockwise").font(.system(size: 12, weight: .semibold))
                }
                Text(rechecking ? String(localized: "Re-checking…") : String(localized: "Re-check"))
                    .font(Face.text(13, .semibold))
            }
            .foregroundStyle(hasLiveShell ? Ink.accent : Ink.meta)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 10)
            .background((hasLiveShell ? Ink.accent : Ink.meta).opacity(0.12),
                        in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                    .strokeBorder((hasLiveShell ? Ink.accent : Ink.meta).opacity(0.18), lineWidth: 1))
        }
        .buttonStyle(.plain)
        .disabled(rechecking || !hasLiveShell)
        .accessibilityIdentifier("islandhooks-recheck")
    }

    @ViewBuilder
    private func resultRow(_ result: RecheckResult) -> some View {
        switch result {
        case .resolved:
            Label {
                Text("Hooks are live — the Vibe Island now shows precise agent status.")
                    .font(Face.text(12, .semibold))
            } icon: {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(Ink.success)
            }
            .foregroundStyle(Ink.primary)
        case .stillMissing:
            Label {
                Text("Run an agent turn in any pane, then re-check.")
                    .font(Face.text(12))
            } icon: {
                Image(systemName: "exclamationmark.circle.fill").foregroundStyle(Ink.warn)
            }
            .foregroundStyle(Ink.secondary)
        case .noChannel:
            Label {
                Text("Connect to a host with tmux to run and verify the hooks.")
                    .font(Face.text(12))
            } icon: {
                Image(systemName: "wifi.slash").foregroundStyle(Ink.warn)
            }
            .foregroundStyle(Ink.secondary)
        }
    }

    // MARK: - Actions

    private func actionButton(title: String, systemImage: String, filled: Bool,
                              enabled: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: systemImage).font(.system(size: 12, weight: .semibold))
                Text(title).font(Face.text(13, .semibold))
            }
            .foregroundStyle(filled ? Ink.screenBG : Ink.accent)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 11)
            .background(
                filled ? AnyShapeStyle(Ink.accent) : AnyShapeStyle(Ink.accent.opacity(0.12)),
                in: RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Metrics.controlRadius, style: .continuous)
                    .strokeBorder(filled ? Ink.accentPressed.opacity(0.36) : Ink.accent.opacity(0.18), lineWidth: 1))
            .opacity(enabled ? 1 : 0.4)
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
    }

    /// Paste + send the installer into the live shell. Visible execution: the
    /// user watches jq/python merge + the backup line scroll by behind the
    /// sheet. Trailing `\r` runs it immediately.
    private func runInTerminal(_ command: String) {
        guard let session else { return }
        // The command types into the ACTIVE pane of the live session — if that
        // pane is running an agent, the installer line lands in the agent's
        // input box and gets submitted as a prompt (burning a turn). Refuse
        // and tell the user to switch to a shell pane first.
        if let control = session.tmuxControl,
           let paneId = control.snapshot.activePaneId ?? control.snapshot.activePanes.first?.id,
           let cmd = control.snapshot.panes[paneId]?.command.lowercased(),
           Self.agentForegrounds.contains(cmd) || control.agentHooks[paneId]?.state != nil {
            runBlockedMessage = "The active pane on \(session.connection.displayName) is running \(cmd) — the command would be typed into the agent's input box. Switch that session to a shell pane, then Run again."
            return
        }
        if let data = (command + "\r").data(using: .utf8) {
            session.sendInput(data)
        }
        dismiss()
    }

    /// Foreground commands that mean "an agent owns this pane's stdin".
    private static let agentForegrounds: Set<String> = [
        "claude", "codex", "gemini", "qwen", "qoder", "factory", "codebuddy",
        "aider", "goose", "opencode", "node",
    ]

    /// Re-check by polling pane hook state over the live tmux control channel.
    /// Triggers a fresh `pollAgentHooks()` (which rebuilds the controller's
    /// `agentHooks` from a one-shot `list-panes -a`), waits a beat for the
    /// round-trip, then resolves if ANY pane now reports a precise `@moshpit_state`.
    private func recheck() {
        guard let controller else { recheckResult = .noChannel; return }
        rechecking = true
        recheckResult = nil
        controller.pollAgentHooks()
        Task { @MainActor in
            // Give the control command time to round-trip and rebuild agentHooks.
            try? await Task.sleep(nanoseconds: 1_200_000_000)
            rechecking = false
            let anyStamped = controller.agentHooks.values.contains { $0.state != nil }
            recheckResult = anyStamped ? .resolved : .stillMissing
        }
    }

    // MARK: - Agents

    /// A coding agent Moshpit can install Vibe Island hooks for. The stamp script
    /// (`~/.moshpit/moshpit-stamp.sh`) is SHARED — agents differ only in where/how
    /// their lifecycle hooks are registered. The read side is agent-agnostic
    /// (any agent that stamps `@moshpit_*` shows up), so adding one is just a new
    /// entry here plus, for a new config format, a builder branch.
    struct IslandAgent: Identifiable, Hashable {
        enum Format { case claudeJSON, codexTOML }
        let id: String          // stamp arg + `@moshpit_agent` label, e.g. "claude"
        let displayName: String
        let configPath: String  // shell expression, e.g. "$HOME/.claude/settings.json"
        let format: Format

        /// Human-readable config location for the footer (no `$HOME`).
        var configHint: String { configPath.replacingOccurrences(of: "$HOME", with: "~") }

        /// JSON-settings agents share Claude Code's hook schema AND stdin payload,
        /// so ONE jq/python merge covers them all — only the path + name change.
        /// Codex uses a TOML config with a different (array-of-tables) shape but
        /// the same stdin payload, so the stamp script is reused as-is.
        static let all: [IslandAgent] = [
            .init(id: "claude",    displayName: "Claude Code", configPath: "$HOME/.claude/settings.json",    format: .claudeJSON),
            .init(id: "codex",     displayName: "Codex",       configPath: "$HOME/.codex/config.toml",       format: .codexTOML),
            .init(id: "gemini",    displayName: "Gemini CLI",  configPath: "$HOME/.gemini/settings.json",    format: .claudeJSON),
            .init(id: "qwen",      displayName: "Qwen Code",   configPath: "$HOME/.qwen/settings.json",      format: .claudeJSON),
            .init(id: "qoder",     displayName: "Qoder",       configPath: "$HOME/.qoder/settings.json",     format: .claudeJSON),
            .init(id: "factory",   displayName: "Factory",     configPath: "$HOME/.factory/settings.json",   format: .claudeJSON),
            .init(id: "codebuddy", displayName: "CodeBuddy",   configPath: "$HOME/.codebuddy/settings.json", format: .claudeJSON),
        ]
    }

    // MARK: - Install command

    static func installCommand(for agent: IslandAgent) -> String {
        switch agent.format {
        case .claudeJSON: return claudeJSONCommand(path: agent.configPath, agent: agent.id)
        case .codexTOML:  return codexTOMLCommand(path: agent.configPath, agent: agent.id)
        }
    }

    /// The SHARED stamp script written to `~/.moshpit/moshpit-stamp.sh`. It stamps
    /// state/agent/since on every fire, and best-effort derives a short
    /// `@moshpit_title` from the hook's stdin JSON via `jq`
    /// (PreToolUse / PermissionRequest → "tool: arg", Notification → message,
    /// UserPromptSubmit → prompt). The title is cleared on done AND whenever no
    /// fresh title is derivable (e.g. no `jq`), so a stale one never lingers; an
    /// `iconv -c` pass drops any UTF-8 char split by the byte-wise `cut`.
    /// Deliberately holds NO single
    /// quotes so it embeds cleanly inside the `<<'EOF'` heredoc of the install
    /// one-liner; jq filters use double quotes (they hold no `$`, so nothing
    /// expands at run time). Without `jq` the title is skipped — state still
    /// stamps. `[ ! -t 0 ]` guards the `cat` from blocking if run manually.
    private static let stampScript = #"""
    #!/bin/sh
    [ -n "$TMUX" ] || exit 0
    ST="$1"; AG="${2:-agent}"; TITLE=""
    if command -v jq >/dev/null 2>&1 && [ ! -t 0 ]; then
      IN=$(cat)
      EV=$(printf "%s" "$IN" | jq -r ".hook_event_name // empty" 2>/dev/null)
      case "$EV" in
        PreToolUse|PermissionRequest)
          TN=$(printf "%s" "$IN" | jq -r ".tool_name // empty" 2>/dev/null)
          AR=$(printf "%s" "$IN" | jq -r ".tool_input.command // .tool_input.file_path // .tool_input.path // .tool_input.pattern // .tool_input.url // empty" 2>/dev/null)
          if [ -n "$AR" ]; then TITLE="$TN: $AR"; else TITLE="$TN"; fi ;;
        Notification)
          TITLE=$(printf "%s" "$IN" | jq -r ".message // empty" 2>/dev/null) ;;
        UserPromptSubmit)
          TITLE=$(printf "%s" "$IN" | jq -r ".prompt // empty" 2>/dev/null) ;;
      esac
      TITLE=$(printf "%s" "$TITLE" | tr "\n" " " | cut -c1-80)
      command -v iconv >/dev/null 2>&1 && TITLE=$(printf "%s" "$TITLE" | iconv -f UTF-8 -t UTF-8 -c 2>/dev/null)
    fi
    tmux set -p -t "$TMUX_PANE" @moshpit_state "$ST" 2>/dev/null
    tmux set -p -t "$TMUX_PANE" @moshpit_agent "$AG" 2>/dev/null
    tmux set -p -t "$TMUX_PANE" @moshpit_since "$(date +%s)" 2>/dev/null
    if [ -n "$TITLE" ] && [ "$ST" != "done" ]; then
      tmux set -p -t "$TMUX_PANE" @moshpit_title "$TITLE" 2>/dev/null
    else
      tmux set -pu -t "$TMUX_PANE" @moshpit_title 2>/dev/null
    fi
    exit 0
    """#

    /// JSON-settings family (Claude Code, Gemini, and the Claude-format forks).
    /// Writes the shared stamp script, backs up the config, then merges our four
    /// hook groups idempotently with jq (python3 fallback). Safe to re-run
    /// (de-dupes on the `moshpit-stamp.sh` substring). Gemini exposes no
    /// `Notification` event — that group is simply inert there (attention falls
    /// back to the BEL heuristic). A single `sh -c '...'` so it pastes as one line.
    private static func claudeJSONCommand(path: String, agent: String) -> String {
        #"""
        sh -c 'set -e; D=$HOME/.moshpit; F=\#(path); mkdir -p "$D" "$(dirname "$F")"; cat > "$D/moshpit-stamp.sh" <<'\''EOF'\''
        \#(stampScript)
        EOF
        chmod 755 "$D/moshpit-stamp.sh"; [ -f "$F" ] && cp "$F" "$F.moshpit.bak.$(date +%s)" || echo "{}" > "$F";
        H="sh ~/.moshpit/moshpit-stamp.sh";
        if command -v jq >/dev/null 2>&1; then
          jq --arg h "$H" '\''
            def grp(c): {hooks:[{type:"command",command:c}]};
            def mgrp(c): {matcher:"*",hooks:[{type:"command",command:c}]};
            def clean(arr;sub): [ (arr // [])[] | select( ([.hooks[]?.command] | map(test(sub)) | any) | not ) ];
            .hooks //= {} |
            .hooks.UserPromptSubmit = ( clean(.hooks.UserPromptSubmit;"moshpit-stamp.sh") + [ grp($h+" working \#(agent)") ] ) |
            .hooks.PreToolUse       = ( clean(.hooks.PreToolUse;"moshpit-stamp.sh")       + [ mgrp($h+" working \#(agent)") ] ) |
            .hooks.Notification     = ( clean(.hooks.Notification;"moshpit-stamp.sh")     + [ mgrp($h+" attention \#(agent)") ] ) |
            .hooks.Stop             = ( clean(.hooks.Stop;"moshpit-stamp.sh")             + [ grp($h+" done \#(agent)") ] )
          '\'' "$F" > "$F.tmp" && mv "$F.tmp" "$F";
        else
          python3 - "$F" "$H" <<'\''PY'\''
        import json,sys
        f,h=sys.argv[1],sys.argv[2]
        try:
            d=json.load(open(f))
        except Exception:
            d={}
        hooks=d.setdefault("hooks",{})
        def grp(c,m=False):
            g={"hooks":[{"type":"command","command":c}]}
            if m: g["matcher"]="*"
            return g
        def merge(ev,cmd,m=False):
            cur=[x for x in hooks.get(ev,[]) if not any("moshpit-stamp.sh" in (hk.get("command","")) for hk in x.get("hooks",[]))]
            cur.append(grp(cmd,m)); hooks[ev]=cur
        merge("UserPromptSubmit",h+" working \#(agent)")
        merge("PreToolUse",h+" working \#(agent)",True)
        merge("Notification",h+" attention \#(agent)",True)
        merge("Stop",h+" done \#(agent)")
        json.dump(d,open(f,"w"),indent=2)
        PY
        fi
        echo "Moshpit hooks installed -> $F (backup alongside). Restart the agent or start a new turn to activate."'
        """#
    }

    /// Codex CLI (`~/.codex/config.toml`). Codex hooks are TOML array-of-tables
    /// (`[[hooks.EventName]]` → `[[hooks.EventName.hooks]]`) and can't be
    /// jq-merged, so we append an idempotent marked block — stripping any prior
    /// Moshpit block with sed first (portable `-i.suffix` form), then re-appending.
    /// Codex's stdin payload matches Claude's, so the shared stamp script and its
    /// `PermissionRequest` title path work unchanged.
    private static func codexTOMLCommand(path: String, agent: String) -> String {
        #"""
        sh -c 'set -e; D=$HOME/.moshpit; F=\#(path); mkdir -p "$D" "$(dirname "$F")"; cat > "$D/moshpit-stamp.sh" <<'\''EOF'\''
        \#(stampScript)
        EOF
        chmod 755 "$D/moshpit-stamp.sh"; [ -f "$F" ] && cp "$F" "$F.moshpit.bak.$(date +%s)" || touch "$F";
        sed -i.moshpittmp '\''/# >>> moshpit vibe island >>>/,/# <<< moshpit vibe island <<</d'\'' "$F" 2>/dev/null; rm -f "$F.moshpittmp";
        cat >> "$F" <<'\''EOT'\''
        # >>> moshpit vibe island >>>
        [[hooks.UserPromptSubmit]]
        [[hooks.UserPromptSubmit.hooks]]
        type = "command"
        command = "sh ~/.moshpit/moshpit-stamp.sh working \#(agent)"

        [[hooks.PreToolUse]]
        [[hooks.PreToolUse.hooks]]
        type = "command"
        command = "sh ~/.moshpit/moshpit-stamp.sh working \#(agent)"

        [[hooks.PermissionRequest]]
        [[hooks.PermissionRequest.hooks]]
        type = "command"
        command = "sh ~/.moshpit/moshpit-stamp.sh attention \#(agent)"

        [[hooks.Stop]]
        [[hooks.Stop.hooks]]
        type = "command"
        command = "sh ~/.moshpit/moshpit-stamp.sh done \#(agent)"
        # <<< moshpit vibe island <<<
        EOT
        echo "Moshpit hooks installed -> $F (backup alongside). Restart Codex or start a new turn to activate."'
        """#
    }
}
