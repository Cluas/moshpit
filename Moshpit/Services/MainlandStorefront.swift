import Foundation
import StoreKit

/// Whether this copy of the app was bought from the China mainland App Store.
///
/// The MIIT APP filing number has to be visible inside the app for the copies
/// that regulation covers: the ones distributed through the mainland store.
/// Everyone else sees a line of Chinese regulatory text they cannot read and
/// did not ask for, so the footer is gated on this answer.
///
/// The storefront — not the device region — is the right signal. A traveller
/// in Shanghai with a US Apple ID downloaded from the US store and is not the
/// regulator's concern; a mainland account roaming abroad still is. Region is
/// kept only as the fallback for when StoreKit has no answer (a fresh
/// simulator, no signed-in account), and DEBUG builds accept a launch argument
/// so a UI test can stand on either side of the line deterministically.
enum MainlandStorefront {

    /// ISO 3166-1 alpha-3 code StoreKit reports for the mainland store.
    static let storefrontCode = "CHN"

    /// ISO 3166-1 alpha-2 code `Locale` reports for the mainland region.
    static let regionCode = "CN"

    /// DEBUG launch argument: `-MOSHPIT_STOREFRONT CHN` (or any other alpha-3
    /// code) replaces the StoreKit answer. Compiled out of Release.
    static let launchArgument = "-MOSHPIT_STOREFRONT"

    /// The one place the decision is made, kept pure so it can be tested
    /// without StoreKit: `storefront` is what StoreKit said (nil when it had no
    /// answer), `region` is the device region, `override` the DEBUG argument.
    static func isMainland(storefront: String?, region: String?, override: String? = nil) -> Bool {
        if let override { return override == storefrontCode }
        if let storefront { return storefront == storefrontCode }
        return region == regionCode
    }

    /// Resolves the answer for this launch. Cheap after the first call:
    /// StoreKit caches the storefront, and this only ever reads it.
    static func isMainland() async -> Bool {
        var override: String?
        #if DEBUG
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: launchArgument), i + 1 < args.count {
            override = args[i + 1]
        }
        #endif
        let storefront = await Storefront.current?.countryCode
        let region = Locale.current.region?.identifier
        return isMainland(storefront: storefront, region: region, override: override)
    }
}
