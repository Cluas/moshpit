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

    // MARK: Failures

    /// The card's text for a failed connect.
    static func failure(kind: FailureKindValue, cause: FailureCauseValue) -> String {
        switch cause {
        case .unreachable:
            return SSHError.connectionFailed.description
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
            return credentials(why)
        case .channel:
            return SSHError.channelOpenFailed.description
        case .moshBootstrap(let why):
            switch why {
            case .timedOut(let seconds):
                return String(localized: "mosh-server didn't start within \(Int(seconds)) seconds. A startup file on the host may be waiting for input.")
            case .unreadable:
                return String(localized: "mosh-server didn't start: it never said where to reach it.")
            }
        case .plain:
            switch kind {
            case .lost:
                return String(localized: "The connection dropped while it was being set up. Try again.")
            case .credentialsLocked:
                return credentials(.locked)
            default:
                return SSHError.underlying(RustCoreFailure()).description
            }
        }
    }

    private static func credentials(_ why: CredentialsWhyValue) -> String {
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
            return SSHError.credentialUnavailable.description
        case .cancelled:
            // Face ID's cancel and the passphrase card's alike (B392).
            return SSHError.credentialCancelled.description
        case .locked:
            return SSHError.credentialLocked.description
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
        case .clientOlder(let client, let server):
            text = HerdrVersionReport(client: client, server: server).notice
        case .serverOlder(let client, let server):
            text = HerdrVersionReport(client: client, server: server).notice
        case .onAnotherDevice:
            text = String(localized: "This pane is open on another device")
        case .inputDropped:
            text = String(localized: "Not attached to the pane yet — input wasn't sent")
        case .couldntSwitch:
            text = String(localized: "Couldn't switch to that pane — nothing was sent")
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

    static func removal(_ outcome: HerdrRemovalValue) -> HerdrControlClient.WorktreeRemoval {
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

    static func restart(_ outcome: HerdrRestartValue) -> HerdrControlClient.RestartOutcome {
        switch outcome {
        case .restarted: return .restarted
        case .failed: return .failed
        case .wouldDowngrade(let client, let server):
            return .wouldDowngrade(HerdrVersionReport(client: client, server: server))
        case .unneeded: return .unneeded
        }
    }
}
