import Foundation
import Testing
@testable import Moshpit

/// Qwen3-ASR's onnxruntime sessions run without a memory arena (B679).
@Suite("Qwen3-ASR session options")
struct Qwen3SessionOptionsTests {
    @Test("The provider names a written file that turns the arena and the memory pattern off")
    func providerWritesOptions() throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("qwen3-options-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let provider = Qwen3ASRModel.provider(writingOptionsTo: directory)
        #expect(provider.hasPrefix("cpu:"))
        let path = String(provider.dropFirst("cpu:".count))
        let lines = try String(contentsOfFile: path, encoding: .utf8).split(separator: "\n")
        #expect(lines.contains("EnableCpuMemArena=0"))
        #expect(lines.contains("EnableMemPattern=0"))
    }

    @Test("A directory that can't take the file falls back to plain cpu, never a missing file")
    func unwritableFallsBack() {
        // sherpa-onnx exits the process on a config file it can't open.
        let missing = URL(fileURLWithPath: "/nonexistent-\(UUID().uuidString)/nowhere", isDirectory: true)
        #expect(Qwen3ASRModel.provider(writingOptionsTo: missing) == "cpu")
    }
}
