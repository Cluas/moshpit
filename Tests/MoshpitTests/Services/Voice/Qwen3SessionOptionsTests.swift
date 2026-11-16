import Foundation
import Testing
@testable import Moshpit

/// Qwen3-ASR's onnxruntime sessions: arena on for phones with 8 GB, off
/// below (B679, B760).
@Suite("Qwen3-ASR session options")
struct Qwen3SessionOptionsTests {
    /// What an 8 GB phone (iPhone 15 Pro and later) and a 6 GB one report.
    static let eightGB: UInt64 = 8_589_934_592
    static let sixGB: UInt64 = 6_442_450_944
    static let fourGB: UInt64 = 4_294_967_296

    private func freshDirectory() throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("qwen3-options-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        return directory
    }

    @Test("Below 7.5 GB the provider names a written file that turns the arena and the memory pattern off",
          arguments: [Qwen3SessionOptionsTests.sixGB, Qwen3SessionOptionsTests.fourGB, 7_499_999_999])
    func smallPhoneWritesOptions(memory: UInt64) throws {
        let directory = try freshDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        #expect(!Qwen3ASRModel.keepsArena(physicalMemory: memory))
        let provider = Qwen3ASRModel.provider(physicalMemory: memory, writingOptionsTo: directory)
        #expect(provider.hasPrefix("cpu:"))
        let path = String(provider.dropFirst("cpu:".count))
        let lines = try String(contentsOfFile: path, encoding: .utf8).split(separator: "\n")
        #expect(lines.contains("EnableCpuMemArena=0"))
        #expect(lines.contains("EnableMemPattern=0"))
    }

    @Test("At 7.5 GB and above the arena stays on: plain cpu, and no file is written",
          arguments: [Qwen3SessionOptionsTests.eightGB, 7_500_000_000, 12_884_901_888])
    func largePhoneKeepsArena(memory: UInt64) throws {
        let directory = try freshDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        #expect(Qwen3ASRModel.keepsArena(physicalMemory: memory))
        #expect(Qwen3ASRModel.provider(physicalMemory: memory, writingOptionsTo: directory) == "cpu")
        #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).isEmpty)
    }

    @Test("A small phone whose directory can't take the file falls back to plain cpu, never a missing file")
    func unwritableFallsBack() {
        // sherpa-onnx exits the process on a config file it can't open.
        let missing = URL(fileURLWithPath: "/nonexistent-\(UUID().uuidString)/nowhere", isDirectory: true)
        #expect(Qwen3ASRModel.provider(physicalMemory: Self.sixGB, writingOptionsTo: missing) == "cpu")
        #expect(Qwen3ASRModel.provider(physicalMemory: Self.eightGB, writingOptionsTo: missing) == "cpu")
    }
}
