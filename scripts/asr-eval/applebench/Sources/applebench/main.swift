// Apple's on-device transcribers over the eval set, file input.
// usage: applebench <set.json> <speech|dictlong|dictshort> <locale> [hotwords-file]
import AVFoundation
import Foundation
import Speech

struct Item: Decodable { let id: String; let wav: String }
let args = CommandLine.arguments
let items = try JSONDecoder().decode([Item].self, from: Data(contentsOf: URL(fileURLWithPath: args[1])))
let mode = args[2]
let locale = Locale(identifier: args[3])
let hotwords: [String] = args.count > 4
    ? try String(contentsOfFile: args[4], encoding: .utf8).split(separator: "\n").map(String.init) : []

func makeModule() async -> any SpeechModule {
    switch mode {
    case "speech":
        let l = await SpeechTranscriber.supportedLocale(equivalentTo: locale) ?? locale
        return SpeechTranscriber(locale: l, transcriptionOptions: [], reportingOptions: [], attributeOptions: [])
    case "dictlong":
        let l = await DictationTranscriber.supportedLocale(equivalentTo: locale) ?? locale
        return DictationTranscriber(locale: l, preset: .progressiveLongDictation)
    default:
        let l = await DictationTranscriber.supportedLocale(equivalentTo: locale) ?? locale
        return DictationTranscriber(locale: l, preset: .progressiveShortDictation)
    }
}

func text(of module: any SpeechModule) async throws -> String {
    var finals = ""
    if let m = module as? SpeechTranscriber {
        for try await r in m.results where r.isFinal { finals += String(r.text.characters) }
    } else if let m = module as? DictationTranscriber {
        for try await r in m.results where r.isFinal { finals += String(r.text.characters) }
    }
    return finals
}

// Assets once.
let probe = await makeModule()
if let request = try await AssetInventory.assetInstallationRequest(supporting: [probe]) {
    FileHandle.standardError.write("installing assets for \(locale.identifier)\n".data(using: .utf8)!)
    try await request.downloadAndInstall()
}

var first = true
for item in items + [items[0]] {
    let module = await makeModule()
    let analyzer = SpeechAnalyzer(modules: [module])
    if !hotwords.isEmpty {
        let context = AnalysisContext()
        context.contextualStrings[.general] = hotwords
        try await analyzer.setContext(context)
    }
    let file = try AVAudioFile(forReading: URL(fileURLWithPath: item.wav))
    let duration = Double(file.length) / file.fileFormat.sampleRate
    let start = Date()
    async let collected = text(of: module)
    if let last = try await analyzer.analyzeSequence(from: file) {
        try await analyzer.finalizeAndFinish(through: last)
    } else {
        await analyzer.cancelAndFinishNow()
    }
    let hyp = try await collected
    let secs = Date().timeIntervalSince(start)
    if first { first = false; continue }  // first pass is the warm-up (re-run at the end)
    let line: [String: Any] = ["id": item.id, "hyp": hyp, "sec": secs, "audio": duration]
    print(String(data: try JSONSerialization.data(withJSONObject: line), encoding: .utf8)!)
}
