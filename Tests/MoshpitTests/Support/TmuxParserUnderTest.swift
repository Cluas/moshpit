import Testing
@testable import Moshpit

/// The tmux control-mode parser a suite runs against, for every
/// `TmuxControlClient` its tests make — directly or inside a
/// `TmuxSessionController` (``TmuxParserBackend/taskOverride``).
///
/// The tmux suites are compiled twice (project.yml): into MoshpitTests,
/// where this is the app's Swift parser, and into MoshpitTmuxRustTests
/// (`MOSHPIT_TMUX_RUST_TESTS`), where it is the Rust one. So every one of
/// their tests — tests added to those files later included — runs against
/// both parsers in one test run, and neither run depends on how the switch
/// happens to be set on the test device.
struct TmuxParserUnderTest: SuiteTrait, TestTrait, TestScoping {
    static var backend: TmuxParserBackend {
        #if MOSHPIT_TMUX_RUST_TESTS
        .rust
        #else
        .swift
        #endif
    }

    /// Down to each test of the suite and of the suites inside it.
    var isRecursive: Bool { true }

    /// Around each test case, not around the suite.
    func scopeProvider(for test: Test, testCase: Test.Case?) -> Self? {
        testCase == nil ? nil : self
    }

    func provideScope(for test: Test, testCase: Test.Case?,
                      performing function: @Sendable () async throws -> Void) async throws {
        if Self.backend == .rust {
            // Without the library the clients would quietly run the Swift
            // parser, and the Rust run would pass without testing anything.
            try #require(TmuxParserBackend.rustAvailable,
                         "this build has no Rust parser (crates/ffi, MoshpitFFI)")
        }
        try await TmuxParserBackend.$taskOverride.withValue(Self.backend) {
            try await function()
        }
    }
}

extension Trait where Self == TmuxParserUnderTest {
    /// Runs the suite's tests against ``TmuxParserUnderTest/backend``.
    static var tmuxParserUnderTest: Self { Self() }
}
