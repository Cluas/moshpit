import Foundation
import Testing
@testable import Moshpit

/// When a Whisper variant on disk counts as installed (B671). WhisperKit's
/// Hub downloader fetches a variant one file at a time, in name order, and a
/// bundle's folder appears with its first small file — so the folders say
/// nothing about whether the weights, each bundle's last and largest file,
/// ever arrived.
@Suite("Whisper model completeness")
struct WhisperModelCompletenessTests {

    /// A scratch variant folder with `bundles`, each holding the files listed.
    private func variant(_ bundles: [String: [String]]) throws -> URL {
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("whisper-\(UUID().uuidString)", isDirectory: true)
        for (bundle, files) in bundles {
            for file in files {
                let url = folder.appendingPathComponent(bundle).appendingPathComponent(file)
                try FileManager.default.createDirectory(
                    at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
                try Data("x".utf8).write(to: url)
            }
        }
        return folder
    }

    /// A bundle as WhisperKit's repo lists it, in the order it arrives.
    private let fullBundle = ["analytics/coremldata.bin", "coremldata.bin", "metadata.json",
                              "model.mil", "weights/weight.bin"]

    @Test("Every bundle with its weights: installed")
    func complete() throws {
        let folder = try variant(["AudioEncoder.mlmodelc": fullBundle,
                                  "MelSpectrogram.mlmodelc": fullBundle,
                                  "TextDecoder.mlmodelc": fullBundle])
        defer { try? FileManager.default.removeItem(at: folder) }
        #expect(WhisperModelStore.isComplete(folder))
    }

    @Test("Paused while the decoder's weights download: not installed (B671)")
    func decoderWeightsMissing() throws {
        // Large v3 Turbo's TextDecoder weights are 591 MB of its 954: the
        // likeliest moment for a pause, a kill or a trip to the background.
        let folder = try variant(["AudioEncoder.mlmodelc": fullBundle,
                                  "MelSpectrogram.mlmodelc": fullBundle,
                                  "TextDecoder.mlmodelc": Array(fullBundle.dropLast())])
        defer { try? FileManager.default.removeItem(at: folder) }
        #expect(!WhisperModelStore.isComplete(folder))
    }

    @Test("A bundle with only its first file: not installed")
    func bundleJustStarted() throws {
        let folder = try variant(["AudioEncoder.mlmodelc": fullBundle,
                                  "MelSpectrogram.mlmodelc": fullBundle,
                                  "TextDecoder.mlmodelc": ["analytics/coremldata.bin"]])
        defer { try? FileManager.default.removeItem(at: folder) }
        #expect(!WhisperModelStore.isComplete(folder))
    }

    @Test("A missing bundle: not installed")
    func bundleMissing() throws {
        let folder = try variant(["AudioEncoder.mlmodelc": fullBundle,
                                  "MelSpectrogram.mlmodelc": fullBundle])
        defer { try? FileManager.default.removeItem(at: folder) }
        #expect(!WhisperModelStore.isComplete(folder))
    }

    @Test("A bundle without a model.mil needs only its coremldata.bin")
    func neuralNetworkBundle() throws {
        let folder = try variant(["AudioEncoder.mlmodelc": fullBundle,
                                  "MelSpectrogram.mlmodelc": ["coremldata.bin", "model.espresso.net"],
                                  "TextDecoder.mlmodelc": fullBundle])
        defer { try? FileManager.default.removeItem(at: folder) }
        #expect(WhisperModelStore.isComplete(folder))
    }
}
