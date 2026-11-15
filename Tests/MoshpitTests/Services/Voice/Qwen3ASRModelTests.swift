import Foundation
import Testing
@testable import Moshpit

/// Qwen3-ASR's pure parts: where long audio is cut, when the decoder read
/// its hotwords back, what counts as a complete download, and the hash the
/// downloader checks. The model itself (0.6 GB, onnxruntime) is not here.
@Suite("Qwen3-ASR model")
struct Qwen3ASRModelTests {

    @Test("Audio under the limit is one piece")
    func shortIsOnePiece() {
        let samples = [Float](repeating: 0.1, count: 16_000 * 5)
        #expect(Qwen3ASRModel.pieces(of: samples) == [0 ..< samples.count])
    }

    @Test("Long audio is cut into contiguous pieces under the limit, at a pause")
    func longIsCutAtAPause() {
        // 50 s of "speech" with one silent half second at 18.0–18.5 s.
        var samples = [Float](repeating: 0.2, count: 16_000 * 50)
        for i in 16_000 * 18 ..< 16_000 * 18 + 8_000 { samples[i] = 0 }
        let pieces = Qwen3ASRModel.pieces(of: samples)
        #expect(pieces.first?.lowerBound == 0)
        #expect(pieces.last?.upperBound == samples.count)
        for (a, b) in zip(pieces, pieces.dropFirst()) { #expect(a.upperBound == b.lowerBound) }
        #expect(pieces.allSatisfy { $0.count <= Qwen3ASRModel.maximumPieceSamples })
        // The first cut falls inside the silence.
        let cut = pieces[0].upperBound
        #expect(cut >= 16_000 * 18 && cut <= 16_000 * 18 + 8_000)
    }

    private let words = ["Claude Code", "Codex", "moshpit", "feature/login", "AppSettings.swift", "Cargo.toml"]

    @Test("The hotword list read back is an echo")
    func echoIsCaught() {
        #expect(Qwen3ASRModel.isPromptEcho(
            "Claude Code Codex moshpit feature/login AppSettings.swift Cargo.toml", hotwords: words))
    }

    @Test("A sentence that uses the hotwords is not an echo")
    func sentenceIsNotEcho() {
        #expect(!Qwen3ASRModel.isPromptEcho("打开 AppSettings.swift 看一下", hotwords: words))
        #expect(!Qwen3ASRModel.isPromptEcho(
            "让 Claude Code 先把 AppSettings.swift 和 Cargo.toml 里的版本号对一下，然后切到 feature/login 分支跑一遍测试",
            hotwords: words))
    }

    @Test("A download is complete only with every file at its size")
    func completeness() throws {
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("qwen3-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        // Sparse files: the sizes without the bytes.
        func place(_ file: Qwen3ASR.RemoteFile, bytes: Int64) throws {
            let url = folder.appendingPathComponent(file.path)
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
            FileManager.default.createFile(atPath: url.path, contents: nil)
            let handle = try FileHandle(forWritingTo: url)
            try handle.truncate(atOffset: UInt64(bytes))
            try handle.close()
        }
        #expect(!Qwen3ASR.isComplete(folder))
        for file in Qwen3ASR.files.dropLast() { try place(file, bytes: file.bytes) }
        #expect(!Qwen3ASR.isComplete(folder))
        let last = try #require(Qwen3ASR.files.last)
        try place(last, bytes: last.bytes - 1)
        #expect(!Qwen3ASR.isComplete(folder))
        try place(last, bytes: last.bytes)
        #expect(Qwen3ASR.isComplete(folder))
    }

    @Test("Files are hashed with SHA-256")
    func hashing() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("sha-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: url) }
        try Data("abc".utf8).write(to: url)
        #expect(try ModelFileDownloader.sha256(of: url)
            == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    }

    @Test("Qwen3-ASR is its own family, everything else is Whisper")
    func families() {
        #expect(WhisperModelStore.family(of: Qwen3ASR.variant) == .qwen3)
        #expect(WhisperModelStore.family(of: "openai_whisper-small") == .whisper)
        #expect(WhisperModelStore.folder(for: Qwen3ASR.variant).lastPathComponent == Qwen3ASR.variant)
        #expect(WhisperModelStore.folder(for: Qwen3ASR.variant) != WhisperModelStore.folder(for: "openai_whisper-small"))
        #expect(WhisperModelStore.resumeCache(for: Qwen3ASR.variant) == WhisperModelStore.folder(for: Qwen3ASR.variant))
    }

    @Test("Every file is pinned to a SHA-256 and the download is about 0.6 GB")
    func manifest() {
        #expect(Qwen3ASR.files.allSatisfy { $0.sha256.count == 64 })
        #expect(Qwen3ASR.totalBytes > 600_000_000 && Qwen3ASR.totalBytes < 650_000_000)
        #expect(Qwen3ASR.url(for: Qwen3ASR.files[0]).absoluteString.contains("/resolve/\(Qwen3ASR.revision)/"))
    }
}
