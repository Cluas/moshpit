import Foundation

/// `moshpit://connection/<uuid>?pane=<paneId>` — the one address for "this
/// agent's pane", written by the app (Live Activity, widget snapshot) and read
/// back by the app's URL handler, the share extension and the Shortcuts intent.
///
/// Build and parse it only through here. tmux pane ids start with `%`, so a
/// hand-interpolated `?pane=%12` is a percent-escape: `URLComponents` decodes it
/// to U+0012 and the tap lands nowhere (single digits only survived because
/// `URL(string:)` re-encodes the invalid `%5` to `%255`). The host is the
/// literal `connection`; the uuid is the path.
public enum AgentDeepLink {
    public static let scheme = "moshpit"

    /// `connectionId` is the connection's `uuidString` (what the agent rows
    /// carry); the uuid overload below is for callers holding a `UUID`.
    public static func string(connectionId: String, paneId: String) -> String {
        var components = URLComponents()
        components.scheme = scheme
        components.host = "connection"
        components.path = "/" + connectionId
        components.queryItems = [URLQueryItem(name: "pane", value: paneId)]
        return components.string ?? "\(scheme)://connection/\(connectionId)"
    }

    public static func string(connectionId: UUID, paneId: String) -> String {
        string(connectionId: connectionId.uuidString, paneId: paneId)
    }

    /// `moshpit://home/needs-you`: Home, at its top, where "Needs you" lists
    /// every agent waiting (B661) — what the island's "+N more" opens.
    public static let needsYou = "\(scheme)://home/needs-you"

    /// Whether `url` is the link to Home's "Needs you".
    public static func isNeedsYou(_ url: URL, schemes: Set<String> = [scheme]) -> Bool {
        guard let urlScheme = url.scheme, schemes.contains(urlScheme) else { return false }
        return url.host() == "home"
    }

    /// The connection and (optional) pane a link points at, or nil when it
    /// isn't a connection link in one of `schemes`.
    public static func parse(_ url: URL, schemes: Set<String> = [scheme])
        -> (connectionId: UUID, paneId: String?)? {
        guard let urlScheme = url.scheme, schemes.contains(urlScheme),
              url.host() == "connection",
              let connectionId = UUID(uuidString: url.lastPathComponent)
        else { return nil }
        return (connectionId, paneId(in: url))
    }

    public static func parse(_ string: String) -> (connectionId: UUID, paneId: String?)? {
        URL(string: string).flatMap { parse($0) }
    }

    private static func paneId(in url: URL) -> String? {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false),
              let item = components.percentEncodedQueryItems?.first(where: { $0.name == "pane" }),
              let raw = item.value
        else { return nil }
        // Snapshots written before this type existed carry the id unencoded
        // (`%12`). No pane id contains a control character, so a decode that
        // produces one means the raw text was the id all along.
        guard let decoded = raw.removingPercentEncoding,
              !decoded.unicodeScalars.contains(where: { $0.properties.generalCategory == .control })
        else { return raw }
        return decoded
    }
}

extension AgentWidgetState.Item {
    /// The pane this row's `deepLink` addresses, or nil when it has none.
    public var target: (connectionId: UUID, paneId: String)? {
        guard let link = deepLink,
              let parsed = AgentDeepLink.parse(link),
              let paneId = parsed.paneId
        else { return nil }
        return (parsed.connectionId, paneId)
    }
}
