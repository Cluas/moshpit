import Foundation
import Testing
@testable import Moshpit

/// X-ASR's download as the model page sees it (B763): half a download to
/// resume, Discard, a finished one that stays until removed.
@Suite("X-ASR download")
struct XASRDownloadTests {
    /// A file of `bytes` at `url`, sparse: the size is what is read.
    static func sized(_ url: URL, _ bytes: Int64) throws {
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        FileManager.default.createFile(atPath: url.path, contents: nil)
        let handle = try FileHandle(forWritingTo: url)
        try handle.truncate(atOffset: UInt64(bytes))
        try handle.close()
    }

    @Test("Half downloaded reads as one to resume; Discard throws it away; a finished one stays until removed")
    func downloadStates() async throws {
        let folder = WhisperModelStore.folder(for: XASR.variant)
        // The store reads the app's own container: never touch a real install.
        guard !FileManager.default.fileExists(atPath: folder.path) else { return }
        defer { try? FileManager.default.removeItem(at: folder) }
        // tokens.txt finished, the encoder part of the way.
        try Self.sized(folder.appendingPathComponent("tokens.txt"), 58_806)
        try Self.sized(folder.appendingPathComponent("encoder.int8.onnx.part"), 1_000_000)
        #expect(!WhisperModelStore.isInstalled(XASR.variant))
        #expect(WhisperModelStore.partialBytes(for: XASR.variant) == 1_058_806)
        let row = WhisperModelStore.catalog().first { $0.id == XASR.variant }
        #expect(row?.isPartial == true)

        let store = WhisperModelStore()
        try await store.discardPartial(variant: XASR.variant)
        #expect(!FileManager.default.fileExists(atPath: folder.path))

        for file in XASR.files { try Self.sized(folder.appendingPathComponent(file.path), file.bytes) }
        #expect(WhisperModelStore.isInstalled(XASR.variant))
        #expect(WhisperModelStore.catalog().first { $0.id == XASR.variant }?.isInstalled == true)
        try await store.discardPartial(variant: XASR.variant)
        #expect(WhisperModelStore.isInstalled(XASR.variant))
        try await store.remove(variant: XASR.variant)
        #expect(!WhisperModelStore.isInstalled(XASR.variant))
    }
}
