import Foundation
import Testing
@testable import Moshpit

/// The skew's wording and direction, as the Home row and the terminal banner
/// read them. Parsing `herdr status` is the Rust core's
/// (crates/herdr/src/version.rs).
@Suite("herdr version report")
struct HerdrVersionReportTests {

    @Test("An older client is a skew the restart can't fix")
    func olderClient() {
        let report = HerdrVersionReport(client: "0.8.0", server: "0.9.5")
        #expect(report.clientIsOlder)
        #expect(report.notice.contains("0.8.0") && report.notice.contains("0.9.5"))
    }

    @Test("The same version both ends is no skew")
    func matched() {
        let report = HerdrVersionReport(client: "0.9.5", server: "0.9.5")
        #expect(!report.clientIsOlder)
    }

    @Test("The server being the older side is a skew the restart fixes")
    func olderServer() {
        let report = HerdrVersionReport(client: "0.9.5", server: "0.8.0")
        #expect(!report.clientIsOlder)
    }

    @Test("Versions compare by number, not by string")
    func numericOrder() {
        #expect(HerdrVersionReport.compare("0.10.0", "0.9.5") == .orderedDescending)
        #expect(HerdrVersionReport.compare("0.8.0", "0.9.5") == .orderedAscending)
        #expect(HerdrVersionReport.compare("1.0", "1.0.0") == .orderedSame)
        #expect(HerdrVersionReport.compare("0.9.5-preview", "0.9.5") == .orderedSame)
    }
}
