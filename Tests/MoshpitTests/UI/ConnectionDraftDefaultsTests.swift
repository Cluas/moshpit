import Foundation
import Testing
@testable import Moshpit

/// B256: a new connection starts from the "Mosh by default" setting (on by
/// default) — in the draft both connection forms open, and in what the NSH
/// form shows of it. The rule is the shared model's
/// (`crates/app/src/connection_form/tests.rs`); these check the host reads
/// the app's setting into it.
@Suite("New connection draft")
@MainActor
struct ConnectionDraftDefaultsTests {

    /// Settings of their own, so a test writes nothing the app keeps.
    private static func scratch() -> (AppSettings, UserDefaults, String) {
        let name = "w5b.draft.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        return (AppSettings(defaults: defaults), defaults, name)
    }

    @Test("mosh by default, as the setting's default says")
    func moshByDefault() {
        let (settings, defaults, name) = Self.scratch()
        defer { defaults.removePersistentDomain(forName: name) }
        #expect(settings.moshByDefault, "the setting is on unless turned off")
        let draft = ConnectionDraft(newWith: settings)
        #expect(draft.useMosh)
        let message = ConnectionFormModel(draft: draft).message(settings: settings, keys: [])
        #expect(message["mosh"] as? Bool == true, "the NSH form shows the draft's protocol")
        var rest = draft
        rest.useMosh = false
        #expect(rest == connectionDraftNew(moshByDefault: false))
    }

    @Test("SSH when the setting is off")
    func sshWhenOff() {
        let (settings, defaults, name) = Self.scratch()
        defer { defaults.removePersistentDomain(forName: name) }
        settings.moshByDefault = false
        #expect(!ConnectionDraft(newWith: settings).useMosh)
        settings.moshByDefault = true
        #expect(ConnectionDraft(newWith: settings).useMosh)
    }

    @Test("an edited connection keeps its own protocol, whatever the setting")
    func editingKeepsItsProtocol() {
        let (settings, defaults, name) = Self.scratch()
        defer { defaults.removePersistentDomain(forName: name) }
        var ssh = ServerConnection()
        ssh.connectionProtocol = .ssh
        #expect(settings.moshByDefault)
        #expect(!ConnectionDraft(ssh).useMosh)
    }
}
