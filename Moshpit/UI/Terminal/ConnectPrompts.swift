import SwiftUI

/// What a connect asks the person on its way in, presented by the app over
/// whatever is frontmost (``PromptOverlay``, B632):
///
/// - the server's host key, before it is trusted: the NSH sheet
///   (``HostKeyNSHPage``, `ui/src/host_key.rs`), whose fingerprints are
///   mono and selectable; a swipe down declines, as its Cancel does;
/// - the passphrase of a key stored behind one, and what the server asks
///   at sign-in beyond the password (B563): the system's alert with a
///   field, as NSH's alerts are the system's but have no field. A secret is
///   typed only into the host's own field (NSH-GAPS #11). Return answers,
///   as the card's field did.
///
/// They replace the SwiftUI cards (`hostKeyPromptCard`,
/// `KeyPassphrasePromptCard`, `SignInQuestionCard`), word for word.
extension View {
    func connectPrompts(
        hostKey: @escaping () -> TerminalViewModel.HostKeyPrompt?,
        passphrase: @escaping () -> TerminalViewModel.KeyPassphrasePrompt?,
        question: @escaping () -> TerminalViewModel.SignInQuestion?
    ) -> some View {
        modifier(ConnectPrompts(hostKey: hostKey, passphrase: passphrase, question: question))
    }
}

private struct ConnectPrompts: ViewModifier {
    let hostKey: () -> TerminalViewModel.HostKeyPrompt?
    let passphrase: () -> TerminalViewModel.KeyPassphrasePrompt?
    let question: () -> TerminalViewModel.SignInQuestion?
    @State private var secret = ""
    @State private var answer = ""

    func body(content: Content) -> some View {
        let key = passphrase()
        let asked = question()
        content
            .appSheet(item: HostKeyNSHPage.binding(hostKey)) { prompt in
                HostKeyNSHPage(prompt: prompt)
            }
            .alert(String(localized: "Key Passphrase"),
                   isPresented: Binding(get: { passphrase() != nil }, set: { _ in }),
                   presenting: key) { prompt in
                SecureField(String(localized: "Passphrase"), text: $secret)
                    .accessibilityIdentifier("passphrase-field")
                Button(String(localized: "Cancel"), role: .cancel) {
                    secret = ""
                    prompt.decide(nil)
                }
                Button(String(localized: "Unlock")) {
                    let typed = secret
                    secret = ""
                    prompt.decide(typed)
                }
                .keyboardShortcut(.defaultAction)
            } message: { prompt in
                Text(prompt.message)
            }
            .alert(Text(asked?.title ?? ""),
                   isPresented: Binding(get: { question() != nil }, set: { _ in }),
                   presenting: asked) { question in
                Group {
                    if question.echo {
                        TextField(String(localized: "Answer"), text: $answer)
                    } else {
                        SecureField(String(localized: "Answer"), text: $answer)
                    }
                }
                .textContentType(.oneTimeCode)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .accessibilityIdentifier("sign-in-answer")
                Button(String(localized: "Cancel"), role: .cancel) {
                    answer = ""
                    question.decide(nil)
                }
                Button(String(localized: "Send")) {
                    let typed = answer
                    answer = ""
                    question.decide(typed)
                }
                .keyboardShortcut(.defaultAction)
            } message: { question in
                Text(verbatim: question.words)
            }
            // A question withdrawn or replaced takes what was typed with
            // it: the next starts empty, and no secret stays behind.
            .onChange(of: key?.id) { secret = "" }
            .onChange(of: asked?.id) { answer = "" }
    }
}

extension TerminalViewModel.KeyPassphrasePrompt {
    /// What its alert says: the card's words.
    var message: String {
        wrong
            ? String(localized: "That passphrase doesn't unlock this key.")
            : String(localized: "The key for \(host) is locked with a passphrase. Enter it once: from then on the Keychain on this device protects the key, as it does every key you import.")
    }
}

extension TerminalViewModel.SignInQuestion {
    /// Its alert's title: the card's.
    var title: String { String(localized: "Sign In to \(host)") }
}

/// The host-key sheet (`ui/src/host_key.rs`): the prompt posted once as
/// `{"op": "hostKey", …}`, the page's `trust` and `decline` answered
/// through `prompt.decide`. If its screen can't run, the sheet says so and
/// a swipe or its Cancel declines.
struct HostKeyNSHPage: View {
    let prompt: TerminalViewModel.HostKeyPrompt
    @State private var failed = false
    /// Answered once: a second press while the sheet leaves is nothing.
    @State private var answered = false

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NSHPage(
                screen: .hostKey,
                attach: { $0.post(json: Self.message(prompt)) },
                onMessage: { message in
                    guard !answered, let trust = Self.decision(message) else { return }
                    answered = true
                    prompt.decide(trust)
                },
                onFailure: { failed = true })
                // One page per prompt: a reused view can never show one
                // prompt's fingerprint while answering another's.
                .id(prompt.id)
                .ignoresSafeArea(.all)
                .background(Ink.pageBG)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
        }
    }

    /// The screen's `{"op": "hostKey", …}` for `prompt`.
    nonisolated static func message(_ prompt: TerminalViewModel.HostKeyPrompt) -> String {
        var fields: [String: Any] = [
            "op": "hostKey", "host": prompt.host, "port": prompt.port, "fingerprint": prompt.fingerprint,
        ]
        if let previous = prompt.previousFingerprint { fields["previous"] = previous }
        if let older = prompt.olderAlgorithm { fields["olderAlgorithm"] = older }
        let data = (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }

    /// Whether the page's `message` trusts the key (true), declines it
    /// (false), or is neither (nil: its `failed` and `retry`, which the
    /// surface handles).
    nonisolated static func decision(_ message: [String: Any]) -> Bool? {
        switch message["ev"] as? String {
        case "trust": true
        case "decline": false
        default: nil
        }
    }

    /// The sheet's item: the prompt `current` reads. A swipe down declines
    /// the prompt the sheet shows, and only while it is still the one
    /// asked: a prompt already answered (its `decide` clears it) is not
    /// answered again.
    static func binding(_ current: @escaping () -> TerminalViewModel.HostKeyPrompt?)
        -> Binding<TerminalViewModel.HostKeyPrompt?> {
        let shown = current()?.id
        return Binding(get: current, set: { new in
            guard new == nil, let asked = current(), asked.id == shown else { return }
            asked.decide(false)
        })
    }
}
