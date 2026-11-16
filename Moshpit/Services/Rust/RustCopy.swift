import Foundation

/// The words for what the Rust core reports as kinds (B202): the core says
/// what happened — a failure's kind and cause, herdr's notices and outcomes
/// — and every sentence a person reads about it is said here, from the
/// app's string catalog, in the person's language. Where the Swift path says
/// the same thing, the same catalog entry says it; what only herdr itself
/// said (an error message, a refusal) is shown as herdr said it.
enum RustCopy {

    // MARK: tmux

    /// What the core refused of tmux, as the Swift controller says it.
    static func tmuxRefusal(_ refused: TmuxRefused) -> String {
        switch refused {
        case .NameHasControlCharacters:
            // `MultiplexerName.refusal`'s words.
            return String(localized: "Names can't contain line breaks or other control characters.")
        }
    }

    /// What the person asked of tmux while nothing carried it (mosh's
    /// control plane down, B401; the connection dialled again, B483)
    /// didn't come through, and it is back.
    static var tmuxActionsDropped: String {
        String(localized: "Your last tmux action didn't go through: the control connection dropped, and it's back now.")
    }

    // MARK: Ends

    /// What a session that ended says, by why; nil when the person
    /// disconnected (nothing to say).
    static func ended(_ reason: EndValue) -> String? {
        switch reason {
        case .disconnected:
            return nil
        case .shellExited:
            return String(localized: "The session ended: the remote shell exited.")
        case .moshServerEnded:
            return String(localized: "The session ended: the remote shell exited or mosh-server was stopped.")
        case .tmuxGone:
            // mosh + tmux: tmux on the host went away under mosh's screen,
            // its own client (B787): the plain mosh line blamed the shell.
            return String(localized: "Session ended: tmux on the host stopped.")
        }
    }

    // MARK: Failures

    /// The card's text for a failed connect of `connection` (nil: one the
    /// copy can't name, which says it without its name). On the jump
    /// host's leg (`leg`, its saved name; F1) the jump host comes first, as
    /// Home's card says it: "the password is wrong" alone would read as
    /// the host's own — and `connection` is then the jump host's. What the
    /// jump host answered about the host behind it names it already.
    static func failure(kind: FailureKindValue, cause: FailureCauseValue,
                        connection: ServerConnection? = nil, leg: String? = nil) -> String {
        let text = words(kind: kind, cause: cause, connection: connection)
        guard let leg, !leg.isEmpty else { return text }
        if case .tunnel = cause { return text }
        return String(localized: "Jump host “\(leg)”: \(text)")
    }

    /// What happened, whichever leg it happened on.
    private static func words(kind: FailureKindValue, cause: FailureCauseValue,
                              connection: ServerConnection?) -> String {
        switch cause {
        case .unreachable(let host, let port, let why):
            return unreachable(host: host, port: port, why: why)
        case .proxy(let proxy, let why):
            if case .unreachable = why { return SSHError.proxyUnreachable(proxy).description }
            return SSHError.proxyFailed.description
        case .handshake:
            return SSHError.handshakeFailed.description
        case .hostKey:
            return SSHError.hostKeyRejected.description
        case .hostKeysLocked:
            return SSHError.hostKeysLocked.description
        case .credentials(let why):
            return credentials(why, connection: connection)
        case .channel:
            return SSHError.channelOpenFailed.description
        case .moshBootstrap(let why):
            switch why {
            case .timedOut(let seconds):
                return String(localized: "mosh-server didn't start within \(Int(seconds)) seconds. A startup file on the host may be waiting for input.")
            case .unreadable:
                return String(localized: "mosh-server didn't start: it never said where to reach it.")
            }
        case .tunnel(let via, let host, let port, let why):
            // The jump host's answer about the host behind it (F1): a host
            // on its network, so never the local-network permission.
            switch why {
            case .prohibited:
                return String(localized: "The jump host “\(via)” doesn't forward connections. Turn on TCP forwarding there (AllowTcpForwarding).")
            case .timedOut:
                return String(localized: "The jump host “\(via)” got no answer from \(host) in time.")
            case .connectFailed, .other:
                return String(localized: "The jump host “\(via)” couldn't reach \(host) on port \(String(port)).")
            }
        case .route(let why):
            // Never a direct dial instead (F1).
            switch why {
            case .deleted:
                return String(localized: "The connection this one goes through was deleted. Pick another in Edit.")
            case .chained:
                return String(localized: "The connection this one goes through goes through another itself. Pick one that connects directly, in Edit.")
            case .unresolved:
                return String(localized: "This connection goes through another, but which one couldn't be told. Pick it again in Edit.")
            }
        case .plain:
            switch kind {
            case .lost:
                return String(localized: "The connection dropped while it was being set up. Try again.")
            case .credentialsLocked:
                return credentials(.locked, connection: connection)
            default:
                return SSHError.underlying(RustCoreFailure()).description
            }
        }
    }

    /// Why nothing answered, in the words of what the person can check
    /// (B740). A host on the local network that didn't answer may be the
    /// permission iOS asks for on the first connect there: denied (or still
    /// unanswered), the connect fails as if the host weren't there, so that
    /// copy names where to allow it. A refusal came from the host itself,
    /// so the permission isn't it.
    static func unreachable(host: String, port: UInt16, why: UnreachableValue) -> String {
        switch why {
        case .refused:
            // A port is a name, not a quantity: 2222, never "2,222".
            return String(localized: "\(host) refused the connection on port \(String(port)). Check the port, and that SSH is running there.")
        case .noAddress:
            // The core's rule, the Home card's too (B801): a `.local` name,
            // looked up on the local network itself.
            if localNetworkMayBeWhy(host: host, why: why) {
                return String(localized: "Couldn't find \(host) on your local network. Check the name, and that Moshpit may use the local network: Settings → Privacy & Security → Local Network.")
            }
            return String(localized: "Couldn't find \(host). Check the host name.")
        case .timedOut, .noRoute, .other:
            if localNetworkMayBeWhy(host: host, why: why) {
                return String(localized: "Couldn't reach \(host) on your local network. Check that it's on, and that Moshpit may use the local network: Settings → Privacy & Security → Local Network.")
            }
            switch why {
            case .timedOut:
                return String(localized: "\(host) didn't answer. Check the address, that the host is on, and your VPN if it sits behind one.")
            case .noRoute:
                return String(localized: "There's no route to \(host) from this network. Check your connection, and your VPN if the host sits behind one.")
            default:
                return SSHError.connectionFailed.description
            }
        }
    }

    private static func credentials(_ why: CredentialsWhyValue, connection: ServerConnection?) -> String {
        switch why {
        case .rejected:
            return SSHError.authenticationFailed.description
        case .methodRejected:
            return SSHError.authMethodRejected.description
        case .interactivePrompts:
            // B393: the server asks its own questions (a one-time code):
            // the rejected method's words, and why.
            return String(localized: "The server rejected this sign-in method. Try a different one (e.g. a key instead of a password). It asks its own questions when you sign in, such as a one-time code, which Moshpit can't answer yet.")
        case .keyUnusable(let key):
            switch key {
            case .encrypted:
                return SSHError.keyNeedsPassphrase.description
            case .unsupported(let algorithm):
                return String(localized: "\(algorithm) keys can't be used to sign in. Pick another key for this connection.")
            case .notOpenSsh, .malformed, .badPublicKey:
                return String(localized: "The private key saved for this connection is damaged, or isn't one Moshpit can read. Import or paste the key again.")
            }
        case .signerFailed:
            return SSHError.underlying(RustCoreFailure()).description
        case .unavailable:
            // Gone, or unreadable with the device unlocked: a phone restored
            // from a backup has the connection and not its keychain item
            // (B782). It used to say "re-sign via SideStore".
            guard let connection else { return SSHError.credentialUnavailable.description }
            return missingCredential(name: connection.displayName, method: connection.authMethod,
                                     managedKey: connection.sshKeyId != nil)
        case .cancelled:
            // Face ID's cancel and the passphrase card's alike (B392).
            return SSHError.credentialCancelled.description
        case .locked:
            return SSHError.credentialLocked.description
        }
    }

    /// What a sign-in says when connection `name`'s saved secret isn't on
    /// this device (B782): its password or the key pasted into it, asked
    /// for again in Edit (Home's Connect opens it with the field ready), or
    /// the managed key it picked (SSH Keys says it is gone, B781).
    static func missingCredential(name: String, method: AuthMethod, managedKey: Bool) -> String {
        switch method {
        case .password:
            String(localized: "The password for \(name) isn't on this iPhone. Enter it again.")
        case .key where managedKey:
            String(localized: "The key \(name) signs in with isn't on this iPhone. Open Edit to pick another key.")
        case .key:
            String(localized: "The private key for \(name) isn't on this iPhone. Paste it again.")
        }
    }

    /// Nothing more to say than "it failed": what the generic copy wraps.
    private struct RustCoreFailure: Error {}

    // MARK: herdr's banner

    /// The banner's text, and its button's.
    static func herdrNotice(_ notice: HerdrNoticeValue?,
                            action: HerdrNoticeActionValue?) -> (String?, String?) {
        let text: String?
        switch notice {
        case nil: text = nil
        case .refused(let message), .mismatch(let message): text = message
        // Which side is older is the core's verdict (crates/herdr
        // version.rs): the remedy each names follows it. A stale copy on
        // PATH needs updating on the host; a stale server is what the
        // restart button beside it fixes.
        case .clientOlder(let client, let server):
            text = String(localized: "herdr on the host's PATH is \(client), its server \(server) — update that copy")
        case .serverOlder(let client, let server):
            text = String(localized: "The host's herdr server is \(server), older than its herdr \(client)")
        case .onAnotherDevice:
            text = String(localized: "This pane is open on another device")
        case .inputDropped:
            text = String(localized: "Not attached to the pane yet — input wasn't sent")
        case .couldntSwitch:
            text = String(localized: "Couldn't switch to that pane — nothing was sent")
        case .switchFailed:
            text = String(localized: "Couldn't switch to that pane")
        case .restarting:
            text = String(localized: "Restarting herdr server…")
        case .restartFailed:
            text = String(localized: "Restart didn't take — on the host, run: herdr server stop && herdr")
        }
        let button: String?
        switch action {
        case nil: button = nil
        case .restartServer: button = String(localized: "Restart herdr server")
        case .retry: button = String(localized: "Retry")
        case .openHere: button = String(localized: "Open here")
        }
        return (text, button)
    }

    // MARK: herdr's outcomes

    static func refusal(_ refusal: HerdrRefusalValue) -> String {
        switch refusal {
        case .unreachable: return String(localized: "Couldn't reach the host")
        case .said(let text): return text
        case .exitStatus(let code): return String(localized: "herdr exited with status \(Int(code))")
        case .cutOff: return String(localized: "herdr's answer was cut off")
        }
    }

    static func task(_ outcome: HerdrTaskOutcomeValue) -> AgentTaskOutcome {
        switch outcome {
        case .started:
            return .started
        case .failed(let failure):
            switch failure {
            case .invalid(let invalid): return .failed(Self.invalid(invalid))
            case .unreachable: return .failed(String(localized: "Couldn't reach the host"))
            case .createFailed: return .failed(String(localized: "Creating the worktree failed"))
            case .unconfirmed(let branch):
                return .failed(String(localized: "herdr didn't confirm the worktree, so no agent was started. Look for a “\(branch)” workspace before trying again."))
            case .hostStoppedAnswering(let branch):
                return .failed(String(localized: "The host stopped answering while the worktree was being made, so no agent was started. Look for a “\(branch)” workspace before trying again."))
            case .herdr(let message): return .failed(message)
            }
        case .incomplete(let incomplete):
            switch incomplete {
            case .noPane:
                return .incomplete(String(localized: "The worktree was created but has no pane"))
            case .cancelled:
                return .incomplete(String(localized: "Cancelled. The worktree is ready, but no agent was started in it."))
            case .launchUnanswered(let agent):
                return .incomplete(String(localized: "The worktree is ready, but the host stopped answering before \(agent) started."))
            case .agentDidNotStart(let agent, let refused):
                let reason = refusal(refused)
                return .incomplete(String(localized: "The worktree is ready, but \(agent) didn't start: \(reason)"))
            case .hostStoppedAnswering:
                return .incomplete(String(localized: "The task is running, but the host stopped answering before the first message was sent."))
            case .messageRefused(let agent, let refused):
                let reason = refusal(refused)
                return .incomplete(String(localized: "The task is running, but \(agent) didn't get the first message: \(reason)"))
            case .agentNotReady(let agent):
                return .incomplete(String(localized: "The task is running, but \(agent) wasn't ready for the first message in time."))
            case .returnFailed(let agent, let refused):
                let reason = refusal(refused)
                return .incomplete(String(localized: "The task is running, but the first message is still in \(agent)'s input: pressing Return failed (\(reason)). Press Return in the pane to send it."))
            }
        }
    }

    private static func invalid(_ invalid: HerdrTaskInvalidValue) -> String {
        switch invalid {
        case .pickRepository: return String(localized: "Pick a repository")
        case .pickAgent: return String(localized: "Pick an agent")
        case .nameBranch: return String(localized: "Name the branch")
        case .branchSpace: return String(localized: "No spaces in a branch name")
        case .branchEdge: return String(localized: "Can't start with “-” or “/”, or end with “/”")
        case .branchDotDot: return String(localized: "No “..” in a branch name")
        case .branchLock: return String(localized: "Can't end with “.lock”")
        case .branchForbidden: return String(localized: "No ~ ^ : ? * [ \\ in a branch name")
        case .branchControl: return String(localized: "No control characters in a branch name")
        }
    }

    static func removal(_ outcome: HerdrRemovalValue) -> RustHerdrController.WorktreeRemoval {
        switch outcome {
        case .removed: return .removed
        case .needsForce: return .needsForce
        case .failed(let why):
            switch why {
            case .unreachable: return .failed(String(localized: "Couldn't reach the host"))
            case .herdr(let message): return .failed(message)
            case .failed: return .failed(String(localized: "Removing the worktree failed"))
            case .cutOff: return .failed(String(localized: "herdr's answer was cut off"))
            case .noAnswer:
                return .failed(String(localized: "The host stopped answering while the worktree was being removed."))
            }
        }
    }

    static func restart(_ outcome: HerdrRestartValue) -> RustHerdrController.RestartOutcome {
        switch outcome {
        case .restarted: return .restarted
        case .failed: return .failed
        case .wouldDowngrade(let client, let server):
            return .wouldDowngrade(client: client, server: server)
        case .unneeded: return .unneeded
        }
    }
}
