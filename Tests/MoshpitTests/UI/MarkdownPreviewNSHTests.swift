import Foundation
import Testing

@testable import Moshpit

/// A Markdown file opened from a path link or the folder sheet: laid out
/// by the NSH page instead of QuickLook's plain text.
@Suite("Markdown preview")
struct MarkdownPreviewNSHTests {
    @Test("a Markdown file goes to the page, anything else to QuickLook")
    func handles() {
        for name in ["README.md", "notes.MD", "doc.markdown", "a.mdx"] {
            #expect(MarkdownPreviewNSHPage.handles(URL(fileURLWithPath: "/tmp/\(name)")), "\(name)")
        }
        for name in ["chart.png", "main.rs", "md", "notes.txt"] {
            #expect(!MarkdownPreviewNSHPage.handles(URL(fileURLWithPath: "/tmp/\(name)")), "\(name)")
        }
    }

    @Test("the page's message: the file's name and text, a message the Rust screen takes")
    func message() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = dir.appendingPathComponent("README.md")
        try Data("# Moshpit\n\n- [x] done\n".utf8).write(to: file)
        let json = MarkdownPreviewNSHPage.message(for: file)
        let fields = try #require(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        #expect(fields["op"] as? String == "markdown")
        #expect(fields["name"] as? String == "README.md")
        #expect(fields["text"] as? String == "# Moshpit\n\n- [x] done\n")
        #expect(NSHSurface.check(screen: NSHScreen.markdownPreview.name, message: fields))
    }

    @Test("a file gone or unreadable is an empty document, not a page that never shows")
    func missing() throws {
        let json = MarkdownPreviewNSHPage.message(for: URL(fileURLWithPath: "/nonexistent/README.md"))
        let fields = try #require(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        #expect(fields["text"] as? String == "")
    }

    @Test("web and mail links open outside the app; a relative link or an anchor doesn't")
    func links() {
        #expect(MarkdownPreviewNSHPage.opens("https://example.com/docs") != nil)
        #expect(MarkdownPreviewNSHPage.opens("mailto:dev@example.com") != nil)
        #expect(MarkdownPreviewNSHPage.opens("docs/setup.md") == nil)
        #expect(MarkdownPreviewNSHPage.opens("#getting-started") == nil)
        #expect(MarkdownPreviewNSHPage.opens("file:///etc/passwd") == nil)
    }

    @Test("the screen's name is the one the Rust side runs it by")
    func name() {
        #expect(NSHScreen.markdownPreview.name == "markdown-preview")
    }
}
