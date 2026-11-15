// Runs WhisperKit the way Moshpit's WhisperKitModel does (same DecodingOptions,
// same zh primer), over every wav in a set.json; prints one JSON line per file.
import Foundation
import WhisperKit

struct Item: Decodable { let id: String; let wav: String }
let args = CommandLine.arguments
let modelFolder = args[1]
let setPath = args[2]
let language: String? = args[3] == "auto" ? nil : args[3]
let usePrimer = args.count > 4 ? args[4] == "primer" : true

func maxRSS() -> Int { var u = rusage(); getrusage(RUSAGE_SELF, &u); return Int(u.ru_maxrss) }

let t0 = Date()
let config = WhisperKitConfig(modelFolder: modelFolder, verbose: false, logLevel: .error,
                              prewarm: false, load: true, download: false)
let kit = try await WhisperKit(config)
let loadSeconds = Date().timeIntervalSince(t0)
FileHandle.standardError.write("loaded in \(loadSeconds)s rss=\(maxRSS())\n".data(using: .utf8)!)

func promptTokens(_ language: String?) -> [Int]? {
    // "primer" applies the app's Chinese mixed-language primer whether the
    // language is pinned to zh or left to auto-detect (B670's question).
    guard usePrimer, let tokenizer = kit.tokenizer, language == nil || language == "zh" else { return nil }
    let primer = "以下是普通话的转写，其中夹杂英文技术词汇，例如 git commit、Docker、SSH。"
    let tokens = tokenizer.encode(text: primer)
    return tokens.isEmpty ? nil : tokens
}

let items = try JSONDecoder().decode([Item].self, from: Data(contentsOf: URL(fileURLWithPath: setPath)))
// One warm-up pass so the first item doesn't carry ANE specialization.
_ = try await kit.transcribe(audioPath: items[0].wav)
for item in items {
    let samples = try AudioProcessor.loadAudioAsFloatArray(fromPath: item.wav)
    let options = DecodingOptions(
        verbose: false, task: .transcribe, language: language, temperature: 0,
        temperatureFallbackCount: 2, usePrefillPrompt: true, detectLanguage: language == nil,
        skipSpecialTokens: true, withoutTimestamps: true, wordTimestamps: false,
        promptTokens: promptTokens(language), compressionRatioThreshold: 2.4,
        logProbThreshold: -1.0, noSpeechThreshold: 0.6, chunkingStrategy: .vad)
    let start = Date()
    let results = try await kit.transcribe(audioArray: samples, decodeOptions: options)
    let secs = Date().timeIntervalSince(start)
    let text = results.map(\.text).joined(separator: " ").trimmingCharacters(in: .whitespacesAndNewlines)
    let line: [String: Any] = ["id": item.id, "hyp": text, "sec": secs, "audio": Double(samples.count) / 16000]
    let data = try JSONSerialization.data(withJSONObject: line)
    print(String(data: data, encoding: .utf8)!)
}
FileHandle.standardError.write("peak_rss=\(maxRSS()) load=\(loadSeconds)\n".data(using: .utf8)!)
