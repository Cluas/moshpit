// The extensions' words in the language Moshpit shows, whatever this
// process's (the "system's") languages are — run by verify-app-language.sh
// over the real catalogs, compiled. Arguments: <catalogs dir> <app language>.
import Foundation

let root = CommandLine.arguments[1]
let app = CommandLine.arguments[2]
let island = Bundle(path: root + "/island.bundle")!
let push = Bundle(path: root + "/push.bundle")!
var failures = 0

func expect(_ label: String, _ got: String, _ want: String) {
    if got != want { failures += 1 }
    print(got == want ? "  ok  " : "  FAIL", label, "→", got, got == want ? "" : "(want \(want))")
}

let system = Locale.preferredLanguages.first ?? "?"
print("system \(system), Moshpit \(app)")
// What each extension wrote before can't be shown here: an extension is
// its own main bundle, and Foundation looks a bundle's strings up in the
// main bundle's language — a command-line tool's, English only. What is
// checked is the new way, which reads `<language>.lproj` (or the keys)
// directly and so is the same in any process.

let want: [String: [String: String]] = [
    "en": [
        "state": "done", "header": "1 needs you · 2 done", "more": "+3 more",
        "stale": "paused — open Moshpit to refresh", "finished": "✓ claude finished",
        "asking": "claude +1 needs you", "minutes": "12 min",
    ],
    "zh-Hans": [
        "state": "已完成", "header": "1 个在等你 · 2 个已完成", "more": "还有 3 个",
        "stale": "已暂停，打开 Moshpit 刷新", "finished": "✓ claude 已完成",
        "asking": "claude +1 在等你", "minutes": "12分钟",
    ],
]
guard let w = want[app] else { fatalError("no expectations for \(app)") }

// The island extension's catalog: a row's state, the header, the overflow, the stale hint.
expect("island state", AppLanguage.string("done", language: app, in: island), w["state"]!)
expect("island header",
       AppLanguage.string("\(1) needs you", language: app, in: island) + " · "
       + AppLanguage.string("\(2) done", language: app, in: island), w["header"]!)
expect("island more", AppLanguage.string("+\(3) more", language: app, in: island), w["more"]!)
expect("island stale", AppLanguage.string("paused — open Moshpit to refresh", language: app, in: island), w["stale"]!)
// The push extension's catalog — which has no en.lproj at all: its English is the keys.
expect("push finished", AppLanguage.string("✓ \("claude") finished", language: app, in: push), w["finished"]!)
expect("push asking", AppLanguage.string("\("claude +1") needs you", language: app, in: push), w["asking"]!)
// A finished turn's duration, in that language's locale.
expect("duration", Duration.seconds(12 * 60).formatted(
    .units(allowed: [.hours, .minutes], width: .abbreviated, maximumUnitCount: 2)
        .locale(AppLanguage.locale(app))), w["minutes"]!)

// The store: nothing recorded is this process's own; recorded is the app's.
UserDefaults(suiteName: AppGroup.id)!.removePersistentDomain(forName: AppGroup.id)
expect("nothing recorded", AppLanguage.current(), AppLanguage.own)
AppLanguage.record(app)
expect("recorded", AppLanguage.current(), app)
UserDefaults(suiteName: AppGroup.id)!.removePersistentDomain(forName: AppGroup.id)

exit(failures == 0 ? 0 : 1)
