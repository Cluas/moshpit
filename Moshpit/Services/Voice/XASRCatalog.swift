import Foundation

// Foundation only: what X-ASR is, where its files come from and the hotword
// vocabulary, apart from the runtime that loads it (`XASRModel.swift`).

// MARK: - Catalog entry

/// X-ASR zh-en, the streaming model (B763): a 0.16B zipformer transducer for
/// Mandarin with English in it, trained to switch between the two inside a
/// sentence, with punctuation and English casing built in.
///
/// It streams, unlike Qwen3-ASR and Whisper: words appear while the person
/// is still talking, and the whole text is there a tenth of a second after
/// they stop. It is small (169 MB, about 0.22 GB resident on one thread), so
/// it is the local model on phones without the memory for Qwen3-ASR, and on
/// the ones with it, it gives Qwen3-ASR's dictation a live preview in place
/// of re-decoding the whole recording every second.
///
/// Measured on real code-switched speech (50 ASCEND clips) it got 11.9% mixed
/// error against Qwen3-ASR's 13.1%, but on developer commands read with a
/// heavy accent it heard far fewer of the terms (48% against 65%), and the
/// pane's hotwords help it little; where memory allows, Qwen3-ASR still
/// writes the text that is inserted.
///
/// The weights are the 480 ms-chunk int8 build k2-fsa publishes for
/// sherpa-onnx (Apache-2.0, from Gilgamesh-J/X-ASR). The release ships them
/// only as one archive, so the files come from Hugging Face copies of that
/// archive's contents (``ModelSources``), every file pinned to the release's
/// own SHA-256.
enum XASR {
    /// The catalog id, and the folder name under the store's base.
    static let variant = "x-asr-zh-en-480ms-int8"

    /// The files of
    /// `sherpa-onnx-x-asr-480ms-streaming-zipformer-transducer-zh-en-punct-int8-2026-06-05`,
    /// pinned by size and hash; where they come from is ``ModelSources``.
    static let files: [Qwen3ASR.RemoteFile] = [
        Qwen3ASR.RemoteFile(path: "encoder.int8.onnx", bytes: 155_278_641,
                            sha256: "908596dcc137a73b95be908ca55e88caa1b3dbbe8027c171615f4b0609c5eb1e"),
        Qwen3ASR.RemoteFile(path: "decoder.onnx", bytes: 11_309_084,
                            sha256: "a1cbc9eac2d5e3fb6617a218c67ad6daaa7f4e0fd225f08b2c22ab0413c8c257"),
        Qwen3ASR.RemoteFile(path: "joiner.int8.onnx", bytes: 2_581_422,
                            sha256: "aedb7fa697b2ab43f20499826fff7c997eea7d67db77be97769aeeeb726e63b3"),
        Qwen3ASR.RemoteFile(path: "tokens.txt", bytes: 58_806,
                            sha256: "b818a60878b9aae978cbb8ad594acbd403d76d1af2e31ef4197c84e2dbdba27c"),
        Qwen3ASR.RemoteFile(path: "bpe.model", bytes: 119_265,
                            sha256: "f87a38025a5fdd1e4e9591f6a44bb81295097ce0b80df6f4ab9f44e52c64ca5f"),
    ]

    static var totalBytes: Int64 { files.reduce(0) { $0 + $1.bytes } }

    /// Where `file` can be fetched from, in the order to try.
    static func urls(for file: Qwen3ASR.RemoteFile) -> [URL] {
        ModelSources.urls(variant: variant, file: file.path)
    }

    /// Every file in place at its size (``Qwen3ASR/isComplete(_:)`` says why
    /// sizes are enough).
    static func isComplete(_ folder: URL) -> Bool {
        files.allSatisfy { file in
            ModelFileDownloader.fileSize(folder.appendingPathComponent(file.path)) == file.bytes
        }
    }

    /// The hotword vocabulary written beside the model on first use
    /// (``SentencePieceVocabulary``).
    static let vocabularyFile = "bpe.vocab"

    /// Hotwords become a graph the beam search walks for every frame: the
    /// pane's best forty, as for Qwen3-ASR.
    static let hotwordLimit = 40

    /// The hotwords as sherpa-onnx reads them from a buffer: one per line,
    /// so a path stays one hotword (its per-stream string splits on `/`).
    ///
    /// Only what the model can write: its tokens have letters, digits, CJK and
    /// `' - .` inside words, and no `/`, `_` or `:` (sherpa-onnx skips a
    /// hotword with a symbol it can't spell, and reads a word starting with
    /// `:` as a boost score), so other ASCII symbols become spaces —
    /// `feature/login` is `feature login`, as the model would write it. Lines
    /// under two characters say nothing and go.
    static func hotwordLines(_ words: [String]) -> String {
        words.compactMap { word -> String? in
            let spelled = String(word.map { character -> Character in
                guard character.isASCII, !character.isLetter, !character.isNumber, !"'-.".contains(character)
                else { return character }
                return " "
            })
            let line = spelled.split(whereSeparator: \.isWhitespace).joined(separator: " ")
            return line.count < 2 ? nil : line
        }.joined(separator: "\n")
    }

    /// One thread: on four, onnxruntime's pool spins between the small
    /// chunks and burns seven times the CPU for 0.03 s less wait after the
    /// last word (measured, 11.3 s of speech: 1.5 CPU seconds on one thread).
    static let threads: Int32 = 1

    /// Silence fed after the last word, so the model's look-ahead sees past
    /// it; with half a second the last words were cut off.
    static let tailSamples = 16_000 * 12 / 10

    /// The model puts a space after Chinese punctuation (`对啊， 大家`);
    /// typed Chinese has none.
    static func tidy(_ text: String) -> String {
        text.replacingOccurrences(of: #"([，。！？、；：])\s+"#, with: "$1", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

// MARK: - Hotword vocabulary

/// The piece list sherpa-onnx's hotword encoder reads (`bpe_vocab`): one
/// `<piece> <score>` per line, from the model's sentencepiece file.
///
/// sherpa-onnx ends the process on a vocabulary line it can't read, so the
/// file is written only from pieces that parse back, and checked before it is
/// handed over.
enum SentencePieceVocabulary {
    /// The pieces and their scores in a sentencepiece `ModelProto`: field 1,
    /// each a `SentencePiece` whose field 1 is the piece and field 2 its
    /// score. Everything else is skipped. Nil when the bytes aren't a model.
    static func pieces(in data: Data) -> [(piece: String, score: Float)]? {
        var reader = ProtoReader(data)
        var out: [(String, Float)] = []
        while !reader.atEnd {
            guard let (field, wire) = reader.key() else { return nil }
            if field == 1, wire == 2 {
                guard let body = reader.bytes() else { return nil }
                var piece = ProtoReader(body)
                var text: String?
                var score: Float = 0
                while !piece.atEnd {
                    guard let (f, w) = piece.key() else { return nil }
                    switch (f, w) {
                    case (1, 2):
                        guard let raw = piece.bytes() else { return nil }
                        text = String(bytes: raw, encoding: .utf8)
                    case (2, 5):
                        guard let bits = piece.fixed32() else { return nil }
                        score = Float(bitPattern: bits)
                    default:
                        guard piece.skip(wire: w) else { return nil }
                    }
                }
                if let text { out.append((text, score)) }
            } else {
                guard reader.skip(wire: wire) else { return nil }
            }
        }
        return out.isEmpty ? nil : out
    }

    /// The vocabulary text, leaving out any piece that would not read back as
    /// one word (whitespace in it, or empty).
    static func text(for pieces: [(piece: String, score: Float)]) -> String {
        pieces.compactMap { entry -> String? in
            guard isWord(entry.piece), entry.score.isFinite else { return nil }
            return "\(entry.piece) \(entry.score)\n"
        }.joined()
    }

    /// True when every line is `<word> <number>`, as sherpa-onnx's reader
    /// insists.
    static func isReadable(_ text: String) -> Bool {
        let lines = text.split(separator: "\n", omittingEmptySubsequences: false).dropLast()
        guard !lines.isEmpty, text.hasSuffix("\n") else { return false }
        return lines.allSatisfy { line in
            let parts = line.split(separator: " ", omittingEmptySubsequences: false)
            return parts.count == 2 && isWord(String(parts[0])) && Float(parts[1]) != nil
        }
    }

    /// `<folder>/bpe.vocab`, written from `<folder>/bpe.model` if it isn't
    /// there yet. Nil when it can't be made safely: the model then runs
    /// without hotwords rather than risking the process.
    static func ensure(in folder: URL) -> URL? {
        let target = folder.appendingPathComponent(XASR.vocabularyFile)
        if let existing = try? String(contentsOf: target, encoding: .utf8), isReadable(existing) {
            return target
        }
        guard let data = try? Data(contentsOf: folder.appendingPathComponent("bpe.model")),
              let pieces = pieces(in: data) else { return nil }
        let text = text(for: pieces)
        guard isReadable(text) else { return nil }
        do {
            try Data(text.utf8).write(to: target, options: .atomic)
        } catch {
            return nil
        }
        return target
    }

    private static func isWord(_ piece: String) -> Bool {
        !piece.isEmpty && !piece.unicodeScalars.contains { CharacterSet.whitespacesAndNewlines.contains($0) }
    }
}

/// Just enough protobuf to walk a sentencepiece model.
private struct ProtoReader {
    private let data: [UInt8]
    private var index = 0

    init(_ data: Data) { self.data = Array(data) }
    init(_ bytes: ArraySlice<UInt8>) { self.data = Array(bytes) }

    var atEnd: Bool { index >= data.count }

    mutating func varint() -> UInt64? {
        var value: UInt64 = 0
        var shift: UInt64 = 0
        while index < data.count, shift < 64 {
            let byte = data[index]
            index += 1
            value |= UInt64(byte & 0x7F) << shift
            if byte & 0x80 == 0 { return value }
            shift += 7
        }
        return nil
    }

    mutating func key() -> (field: UInt64, wire: UInt64)? {
        guard let key = varint() else { return nil }
        return (key >> 3, key & 7)
    }

    mutating func bytes() -> ArraySlice<UInt8>? {
        guard let length = varint(), length <= UInt64(data.count - index) else { return nil }
        defer { index += Int(length) }
        return data[index..<index + Int(length)]
    }

    mutating func fixed32() -> UInt32? {
        guard data.count - index >= 4 else { return nil }
        defer { index += 4 }
        return data[index..<index + 4].reversed().reduce(0) { $0 << 8 | UInt32($1) }
    }

    mutating func skip(wire: UInt64) -> Bool {
        switch wire {
        case 0: return varint() != nil
        case 1: guard data.count - index >= 8 else { return false }; index += 8; return true
        case 2: return bytes() != nil
        case 5: return fixed32() != nil
        default: return false
        }
    }
}

// MARK: - Sources

/// Where pinned model files are fetched from (B765): one table,
/// `Moshpit/Resources/ModelSources.json`, so a second source is a line of
/// data rather than a code change. A file is tried at each source that holds
/// it, in order; what arrives is checked against the hash pinned in the app,
/// so a source can only be slow or gone, never wrong.
enum ModelSources {
    struct Source: Decodable, Equatable {
        /// A revision-pinned folder, ending in `/`.
        let base: String
        let files: [String]
    }

    typealias Table = [String: [Source]]

    /// The table in the app bundle; empty if it is missing or unreadable,
    /// which leaves the model undownloadable rather than guessed at.
    static let bundled: Table = {
        guard let url = Bundle.main.url(forResource: "ModelSources", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return [:] }
        return parse(data) ?? [:]
    }()

    /// The variants' sources in `data`; keys that aren't a list of sources
    /// (`_about`) are skipped.
    static func parse(_ data: Data) -> Table? {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        var table: Table = [:]
        for (variant, value) in object {
            guard let list = value as? [Any],
                  let json = try? JSONSerialization.data(withJSONObject: list),
                  let sources = try? JSONDecoder().decode([Source].self, from: json) else { continue }
            table[variant] = sources
        }
        return table
    }

    static func urls(variant: String, file: String, table: Table = bundled) -> [URL] {
        (table[variant] ?? []).compactMap { source in
            guard source.files.contains(file), source.base.hasPrefix("https://"), source.base.hasSuffix("/")
            else { return nil }
            return URL(string: source.base + file)
        }
    }
}
