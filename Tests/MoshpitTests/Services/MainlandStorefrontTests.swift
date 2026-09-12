import Foundation
import Testing
@testable import Moshpit

/// The rule that decides who sees the MIIT filing number in Settings.
@Suite("Mainland storefront")
struct MainlandStorefrontTests {

    @Test("the mainland store shows it, wherever the device thinks it is")
    func storefrontWins() {
        #expect(MainlandStorefront.isMainland(storefront: "CHN", region: "US"))
        #expect(MainlandStorefront.isMainland(storefront: "CHN", region: nil))
    }

    @Test("another store hides it, even on a device set to China")
    func otherStorefrontHides() {
        #expect(!MainlandStorefront.isMainland(storefront: "USA", region: "CN"))
        #expect(!MainlandStorefront.isMainland(storefront: "HKG", region: "CN"))
    }

    @Test("without a storefront answer the device region decides")
    func regionIsTheFallback() {
        #expect(MainlandStorefront.isMainland(storefront: nil, region: "CN"))
        #expect(!MainlandStorefront.isMainland(storefront: nil, region: "US"))
        #expect(!MainlandStorefront.isMainland(storefront: nil, region: nil))
    }

    @Test("the DEBUG launch argument overrides both")
    func overrideWins() {
        #expect(MainlandStorefront.isMainland(storefront: "USA", region: "US", override: "CHN"))
        #expect(!MainlandStorefront.isMainland(storefront: "CHN", region: "CN", override: "USA"))
    }
}
