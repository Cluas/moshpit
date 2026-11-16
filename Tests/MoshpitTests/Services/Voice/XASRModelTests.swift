import Foundation
import Testing
@testable import Moshpit

/// X-ASR, the streaming model (B763): what is downloaded, which engine a
/// dictation runs on, the order that makes the default, and the hotword
/// vocabulary sherpa-onnx must be able to read.
@Suite("X-ASR streaming model")
struct XASRModelTests {
    static let eightGB: UInt64 = 8_589_934_592
    static let sixGB: UInt64 = 6_442_450_944
    static let fourGB: UInt64 = 4_294_967_296

    @Test("Five pinned files, every one listed by the sources table at a pinned revision")
    func pinnedFiles() {
        #expect(XASR.files.map(\.path) == ["encoder.int8.onnx", "decoder.onnx", "joiner.int8.onnx",
                                           "tokens.txt", "bpe.model"])
        #expect(XASR.totalBytes == 169_347_218)
        #expect(XASR.files.allSatisfy { $0.sha256.count == 64 })
        for file in XASR.files {
            let urls = XASR.urls(for: file)
            #expect(!urls.isEmpty, "no source for \(file.path)")
            for url in urls {
                let text = url.absoluteString
                #expect(text.hasPrefix("https://"))
                #expect(text.hasSuffix("/" + file.path))
                // A revision, not a branch: the bytes behind it can't move.
                #expect(text.range(of: #"/resolve/[0-9a-f]{40}/"#, options: .regularExpression) != nil, "\(text)")
            }
        }
    }

    @Test("Sources are tried in table order; a source that doesn't list a file, or isn't https, is skipped")
    func sourcesTable() throws {
        let json = """
        {"_about": "notes",
         "m": [{"base": "https://a.example.com/r/", "files": ["x.onnx", "t.txt"]},
               {"base": "http://plain.example.com/r/", "files": ["x.onnx"]},
               {"base": "https://b.example.com/r/", "files": ["x.onnx"]}]}
        """
        let table = try #require(ModelSources.parse(Data(json.utf8)))
        #expect(table.keys.sorted() == ["m"])
        #expect(ModelSources.urls(variant: "m", file: "x.onnx", table: table).map(\.absoluteString)
            == ["https://a.example.com/r/x.onnx", "https://b.example.com/r/x.onnx"])
        #expect(ModelSources.urls(variant: "m", file: "t.txt", table: table).count == 1)
        #expect(ModelSources.urls(variant: "m", file: "gone", table: table).isEmpty)
        #expect(ModelSources.urls(variant: "other", file: "x.onnx", table: table).isEmpty)
        #expect(ModelSources.parse(Data("not json".utf8)) == nil)
    }

    @Test("It is its own family, kept beside Qwen3-ASR and resumed from its own folder")
    func family() {
        #expect(WhisperModelStore.family(of: XASR.variant) == .xasr)
        #expect(LocalModelFamily.xasr.isPinnedFiles)
        #expect(!LocalModelFamily.whisper.isPinnedFiles)
        let folder = WhisperModelStore.folder(for: XASR.variant)
        #expect(folder.deletingLastPathComponent() == WhisperModelStore.folder(for: Qwen3ASR.variant).deletingLastPathComponent())
        #expect(WhisperModelStore.resumeCache(for: XASR.variant) == folder)
    }

    @Test("Installed only when every file is there at its size")
    func completeness() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("xasr-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        #expect(!XASR.isComplete(folder))
        // Sparse files of the right sizes: what matters is the size.
        for file in XASR.files.dropLast() {
            let url = folder.appendingPathComponent(file.path)
            FileManager.default.createFile(atPath: url.path, contents: nil)
            let handle = try FileHandle(forWritingTo: url)
            try handle.truncate(atOffset: UInt64(file.bytes))
            try handle.close()
        }
        #expect(!XASR.isComplete(folder))
        let last = try #require(XASR.files.last)
        FileManager.default.createFile(atPath: folder.appendingPathComponent(last.path).path,
                                       contents: Data(count: Int(last.bytes) - 1))
        #expect(!XASR.isComplete(folder))
        FileManager.default.createFile(atPath: folder.appendingPathComponent(last.path).path,
                                       contents: Data(count: Int(last.bytes)))
        #expect(XASR.isComplete(folder))
    }

    @Test("6 GB and more lists Qwen3-ASR first; less lists X-ASR alone, unless Qwen3-ASR is already there")
    func orderByMemory() {
        let none: (String) -> Bool = { _ in false }
        #expect(WhisperModelStore.sherpaOrder(physicalMemory: Self.eightGB, present: none)
            == [Qwen3ASR.variant, XASR.variant])
        #expect(WhisperModelStore.sherpaOrder(physicalMemory: Self.sixGB, present: none)
            == [Qwen3ASR.variant, XASR.variant])
        #expect(WhisperModelStore.sherpaOrder(physicalMemory: Self.fourGB, present: none) == [XASR.variant])
        #expect(WhisperModelStore.sherpaOrder(physicalMemory: Self.fourGB) { $0 == Qwen3ASR.variant }
            == [Qwen3ASR.variant, XASR.variant])
    }

    @Test("X-ASR alone streams; Qwen3-ASR gets X-ASR's preview when it is there; Whisper is unchanged")
    func enginePlan() {
        #expect(LocalDictationPlan.plan(variant: XASR.variant, streamingInstalled: true) == .streaming)
        #expect(LocalDictationPlan.plan(variant: XASR.variant, streamingInstalled: false) == .streaming)
        #expect(LocalDictationPlan.plan(variant: Qwen3ASR.variant, streamingInstalled: true)
            == .batchWithStreamingPreview)
        #expect(LocalDictationPlan.plan(variant: Qwen3ASR.variant, streamingInstalled: false) == .batch)
        #expect(LocalDictationPlan.plan(variant: "openai_whisper-small", streamingInstalled: true) == .batch)
    }

    /// A sentencepiece model in miniature: a trainer spec to skip, then
    /// three pieces, one of which could not be read back.
    static func miniModel() -> Data {
        func varint(_ value: Int) -> [UInt8] {
            var value = value, out: [UInt8] = []
            repeat {
                var byte = UInt8(value & 0x7F)
                value >>= 7
                if value != 0 { byte |= 0x80 }
                out.append(byte)
            } while value != 0
            return out
        }
        func piece(_ text: String, _ score: Float) -> [UInt8] {
            let raw = Array(text.utf8)
            var body: [UInt8] = [0x0A] + varint(raw.count) + raw
            body += [0x15] + withUnsafeBytes(of: score.bitPattern.littleEndian, Array.init)
            body += [0x18, 0x01]
            return [0x0A] + varint(body.count) + body
        }
        var data: [UInt8] = [0x12, 0x03, 0x08, 0x96, 0x01]
        data += piece("▁git", -3.5) + piece("bad piece", -1) + piece("挂", -7.25)
        return Data(data)
    }

    @Test("The vocabulary is read from the model's protobuf, and a piece that can't be read back is left out")
    func vocabulary() throws {
        let pieces = try #require(SentencePieceVocabulary.pieces(in: Self.miniModel()))
        #expect(pieces.map(\.piece) == ["▁git", "bad piece", "挂"])
        #expect(pieces.map(\.score) == [-3.5, -1, -7.25])
        let text = SentencePieceVocabulary.text(for: pieces)
        #expect(text == "▁git -3.5\n挂 -7.25\n")
        #expect(SentencePieceVocabulary.isReadable(text))
        #expect(!SentencePieceVocabulary.isReadable("▁git -3.5\nbad piece -1\n"))
        #expect(!SentencePieceVocabulary.isReadable("▁git\n"))
        #expect(!SentencePieceVocabulary.isReadable(""))
        #expect(SentencePieceVocabulary.pieces(in: Data([0x0A, 0x7F, 0x01])) == nil)
    }

    @Test("No vocabulary can be made from a missing or broken model: the stream then runs without hotwords")
    func vocabularyFallsBack() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("xasr-vocab-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        #expect(SentencePieceVocabulary.ensure(in: folder) == nil)
        try Data([0xFF, 0xFF]).write(to: folder.appendingPathComponent("bpe.model"))
        #expect(SentencePieceVocabulary.ensure(in: folder) == nil)
        try Self.miniModel().write(to: folder.appendingPathComponent("bpe.model"))
        let made = try #require(SentencePieceVocabulary.ensure(in: folder))
        #expect(try String(contentsOf: made, encoding: .utf8) == "▁git -3.5\n挂 -7.25\n")
    }

    @Test("Hotwords go one per line, spelled as the model can write them")
    func hotwordLines() {
        #expect(XASR.hotwordLines(["feature/login", "Claude Code", ":3", "max_tokens", "AppSettings.swift",
                                   "localhost:8080", "it's", "git-lfs", "部署 staging"])
            == "feature login\nClaude Code\nmax tokens\nAppSettings.swift\nlocalhost 8080\nit's\ngit-lfs\n部署 staging")
        #expect(XASR.hotwordLines([]) == "")
        #expect(XASR.hotwordLines(["/", "x"]) == "")
    }

    @Test("The space the model puts after Chinese punctuation goes")
    func tidy() {
        #expect(XASR.tidy("对啊， 大家最近都是觉得 AI 有一天能够 replace 人类。 ") == "对啊，大家最近都是觉得 AI 有一天能够 replace 人类。")
        #expect(XASR.tidy("Please refactor this, then run it.") == "Please refactor this, then run it.")
    }
}
