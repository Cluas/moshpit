import SwiftUI
import UIKit

/// A Markdown file of the host, laid out by NSH (`ui/src/terminal/markdown.rs`)
/// instead of QuickLook, which shows a `.md` as its source. The file is the
/// phone's copy (``RemotePreviewCache``); the host reads it and posts
/// `{"op": "markdown", "name", "text"}`. Done closes the sheet, Share hands
/// the file to another app, and a web or mail link opens outside the app.
struct MarkdownPreviewNSHPage: View {
    let url: URL

    @Environment(\.dismiss) private var dismiss
    @State private var failed = false

    var body: some View {
        if failed {
            NSHSheetFailed { failed = false }
        } else {
            NSHPage(
                screen: .markdownPreview,
                attach: { $0.post(json: Self.message(for: url)) },
                onMessage: handle,
                onFailure: { failed = true })
                .ignoresSafeArea(.all)
                .background(Ink.pageBG)
        }
    }

    /// The page's `{"op": "markdown", …}`: the file's name and its text,
    /// read leniently (a stray byte that isn't UTF-8 shows as U+FFFD), its
    /// first ``limit`` bytes only, so a huge file can't stall the page.
    nonisolated static func message(for url: URL) -> String {
        var text = ""
        if let handle = try? FileHandle(forReadingFrom: url) {
            let data = (try? handle.read(upToCount: limit)) ?? Data()
            try? handle.close()
            text = String(decoding: data, as: UTF8.self)
        }
        let fields: [String: Any] = ["op": "markdown", "name": url.lastPathComponent, "text": text]
        let data = (try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys])) ?? Data()
        return String(decoding: data, as: UTF8.self)
    }

    /// The most of a file the page lays out: 2 MB.
    nonisolated static let limit = 2 << 20

    /// Whether `url` is a Markdown file, shown here rather than by QuickLook.
    nonisolated static func handles(_ url: URL) -> Bool {
        ["md", "markdown", "mdown", "mkd", "mdx"].contains(url.pathExtension.lowercased())
    }

    /// A link the page may open outside the app: the web or mail. A
    /// relative link (another file of the host) or an anchor opens nothing.
    nonisolated static func opens(_ link: String) -> URL? {
        guard let url = URL(string: link), let scheme = url.scheme?.lowercased(),
              ["http", "https", "mailto"].contains(scheme) else { return nil }
        return url
    }

    private func handle(_ message: [String: Any]) {
        switch message["ev"] as? String {
        case "close":
            dismiss()
        case "share":
            RemoteShareSheet.present(url)
        case "link":
            if let link = message["url"] as? String, let target = Self.opens(link) {
                UIApplication.shared.open(target)
            }
        default:
            break
        }
    }
}

/// A local copy that opens in ``MarkdownPreviewNSHPage``.
struct MarkdownPreviewItem: Identifiable {
    let url: URL
    var id: URL { url }
}

extension View {
    /// `.quickLookPreview($url, in: items)`, except that a Markdown file
    /// opens laid out (``MarkdownPreviewNSHPage``) in a sheet of its own:
    /// QuickLook shows one as its source. `url` stays the one binding both
    /// read and clear, so whoever watches it for the preview going away
    /// still hears it.
    func remotePreview(_ url: Binding<URL?>, in items: [URL]? = nil) -> some View {
        let quickLook = Binding<URL?>(
            get: { url.wrappedValue.flatMap { MarkdownPreviewNSHPage.handles($0) ? nil : $0 } },
            set: { url.wrappedValue = $0 })
        let markdown = Binding<MarkdownPreviewItem?>(
            get: {
                url.wrappedValue.flatMap { MarkdownPreviewNSHPage.handles($0) ? MarkdownPreviewItem(url: $0) : nil }
            },
            set: { if $0 == nil { url.wrappedValue = nil } })
        return Group {
            if let items {
                quickLookPreview(quickLook, in: items.filter { !MarkdownPreviewNSHPage.handles($0) })
            } else {
                quickLookPreview(quickLook)
            }
        }
        .appSheet(item: markdown) { MarkdownPreviewNSHPage(url: $0.url) }
    }
}
