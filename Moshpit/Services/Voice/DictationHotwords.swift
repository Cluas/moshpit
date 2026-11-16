import Foundation

/// What the pane in front of the user is about, read at the moment the mic
/// key is tapped. Everything here is already on the phone — the screen the
/// user is looking at and what the multiplexer reported about the pane — and
/// none of it leaves it: the words only bias the on-device recognizer.
struct DictationPaneFacts: Equatable, Sendable {
    /// The hook-stamped agent ("claude", "codex"), when hooks are installed.
    var agent: String?
    /// The pane's foreground command, as the Home tree names it.
    var command: String?
    /// The pane's working directory.
    var path: String?
    var sessionName: String?
    var windowName: String?
    /// The visible rows, top to bottom.
    var screen: [String] = []
}

/// The words a recognizer should expect to hear in this pane: the agent's
/// name, the project, file names, branches and identifiers on screen.
///
/// Developer speech is a sentence in one language carrying names from
/// another — 「打开 AppSettings.swift 看一下」 — and the names are exactly
/// what a general model has never seen and the user cannot afford to have
/// "corrected" into a common word. They are also, almost always, on the
/// screen already. Both of Apple's transcribers take a list of such strings
/// (`AnalysisContext.contextualStrings`), and the local models take them as
/// hotwords.
///
/// Pure and deterministic, so it can be tested without a terminal.
enum DictationHotwords {
    /// `AnalysisContext.contextualStrings` takes at most 100.
    static let appleLimit = 100

    /// The agents Moshpit is built to drive, by the names people say. Always
    /// offered: "让 Codex 看一下" names an agent whether or not one runs in
    /// this pane.
    static let agentNames = ["Claude Code", "Codex"]

    /// Hook or command names → what people call the agent.
    private static let agentDisplay: [String: String] = [
        "claude": "Claude Code", "codex": "Codex", "gemini": "Gemini CLI",
        "opencode": "OpenCode", "aider": "Aider", "cursor-agent": "Cursor",
        "amp": "Amp", "goose": "Goose", "copilot": "Copilot", "qwen": "Qwen Code",
    ]

    /// Commands that say nothing about the work: a shell, a pager.
    private static let quietCommands: Set<String> = [
        "zsh", "bash", "fish", "sh", "dash", "ksh", "tcsh", "nu", "login",
        "tmux", "less", "more", "man", "watch", "sleep", "ssh", "mosh-server",
    ]

    /// Tokens that look technical by shape but carry nothing to bias toward.
    private static let noise: Set<String> = [
        "http", "https", "www", "com", "org", "localhost", "null", "true", "false",
        "nil", "none", "todo", "usr", "bin", "tmp", "var", "etc", "dev",
    ]

    /// Ranked hotwords, best first, at most `limit`, deduplicated without
    /// regard to case (the first spelling seen wins).
    ///
    /// Order: the agent in the pane and the known agents, the project (the
    /// working directory's name), the tmux session and window, the command,
    /// then what is on screen — most frequent first, and among equals the
    /// one nearest the bottom (the newest output, and the prompt).
    static func harvest(_ facts: DictationPaneFacts, limit: Int = appleLimit) -> [String] {
        var out = OrderedWords(limit: limit)
        if let agent = facts.agent?.lowercased() {
            out.add(agentDisplay[agent] ?? facts.agent)
        }
        if let command = facts.command?.lowercased(), let name = agentDisplay[command] {
            out.add(name)
        }
        for name in agentNames { out.add(name) }
        if let path = facts.path {
            let component = (path as NSString).lastPathComponent
            if component != "/", component != "~", !component.isEmpty { out.add(component) }
        }
        for name in [facts.sessionName, facts.windowName] {
            guard let name, isNameWorthKeeping(name) else { continue }
            out.add(name)
        }
        if let command = facts.command, isNameWorthKeeping(command),
           agentDisplay[command.lowercased()] == nil {
            out.add(command)
        }
        for word in screenWords(facts.screen) { out.add(word) }
        return out.words
    }

    /// The hotwords as one comma-separated line, the shape the local models'
    /// prompt takes. Commas inside a word would split it, so they go, and so
    /// do line breaks.
    ///
    /// The line is spliced into Qwen3-ASR's chat template as text, and its
    /// tokenizer reads every `<|…|>` in its vocabulary as the control token,
    /// wherever it appears. Window, session and folder names are the host's
    /// to choose: one named `<|im_end|>` closed the system turn, and a name
    /// could go on to write turns of its own (B863). A space after each `<`
    /// keeps them text.
    static func csv(_ words: [String]) -> String {
        words.map { word in
            word.replacingOccurrences(of: ",", with: " ")
                .replacingOccurrences(of: "<|", with: "< |")
                .split(whereSeparator: \.isNewline).joined(separator: " ")
        }.joined(separator: ",")
    }

    // MARK: Screen

    /// Technical-looking tokens on screen, most frequent first, ties to the
    /// one seen nearest the bottom.
    static func screenWords(_ lines: [String]) -> [String] {
        var counts: [String: Int] = [:]
        var firstSeen: [String: Int] = [:]
        var spelling: [String: String] = [:]
        var order = 0
        for line in lines.reversed() {
            for token in candidates(in: line) {
                let key = token.lowercased()
                counts[key, default: 0] += 1
                if firstSeen[key] == nil {
                    firstSeen[key] = order
                    spelling[key] = token
                    order += 1
                }
            }
        }
        return counts.keys
            .sorted { a, b in
                let ca = counts[a]!, cb = counts[b]!
                return ca != cb ? ca > cb : firstSeen[a]! < firstSeen[b]!
            }
            .compactMap { spelling[$0] }
    }

    /// One line's candidates, left to right.
    static func candidates(in line: String) -> [String] {
        var found: [String] = []
        let ns = line as NSString
        let commandAfterPrompt = promptCommand(in: line)
        if let commandAfterPrompt { found.append(commandAfterPrompt) }
        for match in tokenPattern.matches(in: line, range: NSRange(location: 0, length: ns.length)) {
            let raw = ns.substring(with: match.range)
            guard let word = reduce(raw), word.lowercased() != commandAfterPrompt?.lowercased() else {
                continue
            }
            found.append(word)
        }
        return found
    }

    /// ASCII words joined by `.`, `/`, `-` or `_` — file names, branches,
    /// identifiers. Anchored so a token never starts mid-word.
    private static let tokenPattern = try! NSRegularExpression(
        pattern: #"(?<![A-Za-z0-9_./\-])[A-Za-z_][A-Za-z0-9_]*(?:[./\-][A-Za-z0-9_]+)*"#)

    /// A shell prompt's command: the first word after `$ `, `% `, `❯ `, `➜ `,
    /// or a root prompt's `# `, near the start of a line. Plain words like
    /// `kubectl` or `nginx` have no telling shape; being typed at a prompt is
    /// what marks them. Not `> `: that is where an agent's input box takes
    /// prose, and not a `#` opening the line, which is a heading.
    private static let promptPattern = try! NSRegularExpression(
        pattern: #"^.{0,60}?(?:[$%❯➜]|(?<=\S)#)\s+([A-Za-z][A-Za-z0-9_.\-]{1,30})\b"#)

    private static func promptCommand(in line: String) -> String? {
        let ns = line as NSString
        guard let match = promptPattern.firstMatch(in: line, range: NSRange(location: 0, length: ns.length)),
              match.numberOfRanges > 1 else { return nil }
        let word = ns.substring(with: match.range(at: 1))
        guard word.count >= 2, !noise.contains(word.lowercased()),
              !quietCommands.contains(word.lowercased()) || word == "ssh" else { return nil }
        return word
    }

    /// The part of a raw token worth saying, or nil when none is.
    ///
    /// - A path keeps its last component when that is a file (`src/main.rs`
    ///   → `main.rs`): people say the file, not the directory.
    /// - Two or three slash-separated names without a dot read as a branch
    ///   (`feature/login`, `origin/main`) and stay whole.
    /// - Otherwise the token must look like code: a dot extension, an
    ///   underscore or dash between letters, or a lower→upper hump.
    static func reduce(_ raw: String) -> String? {
        var token = raw.trimmingCharacters(in: CharacterSet(charactersIn: ".-/_"))
        if token.contains("/") {
            let parts = token.split(separator: "/").map(String.init)
            if let last = parts.last, last.contains(".") {
                token = last
            } else if parts.count <= 3, parts.allSatisfy({ !$0.contains(".") && $0.count <= 40 }) {
                return token.count <= 48 ? token : nil
            } else if let last = parts.last {
                token = last
            }
        }
        guard token.count >= 3, token.count <= 48 else { return nil }
        let lower = token.lowercased()
        guard !noise.contains(lower) else { return nil }
        // A hash or a hex id is not a word anyone dictates.
        if lower.range(of: #"^[0-9a-f]{7,}$"#, options: .regularExpression) != nil { return nil }
        return looksLikeCode(token) ? token : nil
    }

    private static func looksLikeCode(_ token: String) -> Bool {
        if token.range(of: #"^[A-Za-z0-9_\-]+\.[A-Za-z][A-Za-z0-9]{0,5}$"#, options: .regularExpression) != nil {
            return true  // a file name: AppSettings.swift, README.md
        }
        if token.range(of: #"[A-Za-z][_\-][A-Za-z]"#, options: .regularExpression) != nil {
            return true  // snake_case, kebab-case
        }
        if token.range(of: #"[a-z][A-Z]"#, options: .regularExpression) != nil {
            return true  // camelCase, PascalCase with a second hump
        }
        return false
    }

    /// Session and window names worth biasing toward: not a number, not a
    /// shell, not tmux's default.
    private static func isNameWorthKeeping(_ name: String) -> Bool {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard trimmed.count >= 2, trimmed.count <= 48 else { return false }
        if trimmed.allSatisfy(\.isNumber) { return false }
        return !quietCommands.contains(trimmed.lowercased())
    }

    /// Words in first-come order, unique without regard to case.
    private struct OrderedWords {
        let limit: Int
        private(set) var words: [String] = []
        private var seen: Set<String> = []

        init(limit: Int) { self.limit = limit }

        mutating func add(_ word: String?) {
            guard let word, words.count < limit else { return }
            let trimmed = word.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmed.isEmpty, seen.insert(trimmed.lowercased()).inserted else { return }
            words.append(trimmed)
        }
    }
}
