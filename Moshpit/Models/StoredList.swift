import Foundation
import MoshpitKit

/// One JSON array under one `UserDefaults` key, read an entry at a time.
///
/// Every list store here — connections, SSH keys, shortcuts, terminal and app
/// themes — used to decode its array in one piece, and one piece it could not
/// decode read as an EMPTY list. The store believed it, and the next add, edit
/// or delete saved that empty list over every entry the user had. Any schema
/// drift was enough: a field a newer build started requiring, an enum case an
/// older build never heard of (the files chip, rolled back to a build without
/// it), one corrupt byte. TestFlight keeps two builds a tap apart.
///
/// So the read is per entry, and nothing it cannot read is ever dropped:
///  * entries this build decodes are the list it shows ("elements");
///  * entries it cannot are kept exactly as stored and written back after the
///    others on every save (``Contents/carried``) — so the build that wrote
///    them finds them again, and a save never shortens the list on its own;
///  * a value that is not a JSON array at all cannot be carried that way, so
///    it is copied untouched to `<key>.backup` before anything may be written
///    over it — and so is the first list that had an entry skipped, so an
///    untouched original exists for that case too (``secure(_:in:)``).
/// Every skip is logged (`Log.store`).
///
/// What the stores keep from ``DefaultsReadiness``: nothing here writes unless
/// the caller has established that the defaults can be trusted. A sealed file
/// reads as nothing stored, which is complete and needs no backup.
struct StoredList<Element: Codable> {
    let key: String

    /// Where values this build could not read are kept, untouched.
    var backupKey: String { key + ".backup" }

    /// Distinct non-list values the backup holds at most, oldest dropped
    /// first. Each takes something outside this app writing garbage under the
    /// key; the cap only bounds a pathological loop of that.
    static var backupLimit: Int { 5 }

    /// What one read of the key found.
    struct Contents {
        /// The entries this build decoded, in stored order.
        var elements: [Element] = []
        /// The entries it could not, as stored (JSON values). Written back
        /// after ``elements`` on every save.
        var carried: [Any] = []
        /// Exactly what the key held; nil when nothing was stored.
        var stored: Any?
        /// False when ``stored`` is not a JSON array at all — then none of it
        /// can be carried entry by entry.
        var isList = true

        /// Everything stored was read: nothing skipped, nothing unreadable.
        var isComplete: Bool { carried.isEmpty && isList }
    }

    /// Read the key, decoding each entry on its own with `decoder`.
    func read(from defaults: UserDefaults, using decoder: JSONDecoder = JSONDecoder()) -> Contents {
        var contents = Contents()
        guard let stored = defaults.object(forKey: key) else { return contents }
        contents.stored = stored
        guard let data = stored as? Data,
              let entries = (try? StoredJSON.object(with: data)) as? [Any] else {
            contents.isList = false
            Log.store.error("\(key, privacy: .public): stored value is not a list (\(Self.describe(stored), privacy: .public)); nothing loaded, value kept")
            return contents
        }
        // JSONSerialization refuses 1e400 but reads -1e400 as -infinity, and
        // writing that back out — the next step, for every entry — raises an
        // Objective-C exception: one such number anywhere in the list, even
        // in a field nobody reads, crashed every launch (B154). Out of
        // Double's range is out of range for either sign, as moshpit-store
        // reads it: the value is no list, kept untouched like any other.
        guard !Self.containsNonFiniteNumber(entries) else {
            contents.isList = false
            Log.store.error("\(key, privacy: .public): stored list holds a number out of range; nothing loaded, value kept")
            return contents
        }
        for (index, entry) in entries.enumerated() {
            do {
                // Wrapped in a one-entry array: a bare string or number is a
                // JSON fragment, which JSONSerialization won't write alone.
                let single = try JSONSerialization.data(withJSONObject: [entry])
                guard let element = try decoder.decode([Element].self, from: single).first else {
                    throw DecodingError.valueNotFound(Element.self, .init(
                        codingPath: [], debugDescription: "empty entry"))
                }
                contents.elements.append(element)
            } catch {
                contents.carried.append(entry)
                Log.store.error("\(key, privacy: .public): entry \(index) of \(entries.count) unreadable, kept as stored: \(String(describing: error), privacy: .public)")
            }
        }
        return contents
    }

    /// Make sure what `contents` was read from survives the next save over
    /// it, and say whether that save is now safe.
    ///
    /// A complete read needs nothing. A list with entries skipped carries them
    /// through every save, so one untouched copy of the FIRST such list is all
    /// the backup it needs. A value that is not a list at all is the only
    /// copy of whatever it held, so each distinct one is kept. Only call this
    /// once the defaults are known to be readable — it writes.
    func secure(_ contents: Contents, in defaults: UserDefaults) -> Bool {
        guard !contents.isComplete, let stored = contents.stored else { return true }
        var backups = defaults.array(forKey: backupKey) ?? []
        let alreadyKept = backups.contains { ($0 as AnyObject).isEqual(stored) }
        if alreadyKept || (contents.isList && !backups.isEmpty) { return true }
        backups.append(stored)
        if backups.count > Self.backupLimit {
            backups.removeFirst(backups.count - Self.backupLimit)
        }
        defaults.set(backups, forKey: backupKey)
        // Read back rather than assumed: a store may write over the original
        // only once this copy exists.
        let kept = defaults.array(forKey: backupKey)?
            .contains { ($0 as AnyObject).isEqual(stored) } ?? false
        if kept {
            Log.store.notice("\(key, privacy: .public): value as stored copied to \(backupKey, privacy: .public)")
        } else {
            Log.store.error("\(key, privacy: .public): could not keep a copy in \(backupKey, privacy: .public); not writing over it")
        }
        return kept
    }

    /// `elements` encoded with `encoder`, then `carried` exactly as it was
    /// read. nil when encoding fails — the caller writes nothing then.
    func encode(_ elements: [Element], carrying carried: [Any],
                using encoder: JSONEncoder = JSONEncoder()) -> Data? {
        guard let data = try? encoder.encode(elements) else { return nil }
        guard !carried.isEmpty else { return data }
        guard let decoded = (try? StoredJSON.object(with: data)) as? [Any] else { return nil }
        return try? JSONSerialization.data(withJSONObject: decoded + carried)
    }

    private static func describe(_ value: Any) -> String {
        if let data = value as? Data { return "\(data.count) bytes" }
        return String(describing: type(of: value))
    }

    /// Whether `value`, as JSONSerialization parsed it, holds a number JSON
    /// can't carry — what it makes of `-1e400` — anywhere inside it.
    static func containsNonFiniteNumber(_ value: Any) -> Bool {
        switch value {
        case let object as [String: Any]:
            return object.values.contains(where: containsNonFiniteNumber)
        case let array as [Any]:
            return array.contains(where: containsNonFiniteNumber)
        case let number as NSNumber:
            // A Bool is an NSNumber too; it is finite.
            return !number.doubleValue.isFinite
        default:
            return false
        }
    }
}

/// `JSONSerialization.jsonObject(with:)` as the stores read with it, without
/// one of its rules (B155): it drops a leading U+FEFF from every string it
/// parses, keys included — an escaped `\uFEFF` as well. The stores parse
/// every list through it on every read and every save that carries an entry
/// (``StoredList``, `ConnectionStore.addingRollbackFields`,
/// `KeychainKnownHostsStore`'s salvage), so a name, host, user or comment
/// that starts with one lost it at the next launch, and the next save wrote
/// the change: a host name changed this way connects somewhere else.
/// moshpit-store reads the list by JSONSerialization's rules with every
/// string as stored (`stored_list::serialization_tree`); so does this.
///
/// Every other rule stays JSONSerialization's own — what it refuses, its
/// numbers, its objects — because it still parses: each U+FEFF inside a
/// string goes in as a private-use scalar the data holds nowhere else, and
/// comes back out as U+FEFF in every string and key of the result. A value
/// holding no U+FEFF at all is parsed as it always was.
enum StoredJSON {
    static func object(with data: Data) throws -> Any {
        let bytes = [UInt8](data)
        guard let stand = standIn(for: bytes) else {
            return try JSONSerialization.jsonObject(with: data)
        }
        return restored(try JSONSerialization.jsonObject(with: Data(masked(bytes, with: stand))), stand)
    }

    /// A private-use scalar to stand in for U+FEFF while JSONSerialization
    /// parses `bytes` — one the data holds neither as itself nor as an
    /// escape — or nil when there is no U+FEFF in any string to protect.
    private static func standIn(for bytes: [UInt8]) -> Unicode.Scalar? {
        var bom = false
        var held = Set<UInt32>()
        // What the strings could hold: every scalar as written, and every
        // `\uXXXX` escape.
        for scalar in String(decoding: bytes, as: UTF8.self).unicodeScalars {
            if scalar.value == 0xFEFF { bom = true }
            held.insert(scalar.value)
        }
        var index = 0
        while index + 5 < bytes.count {
            if bytes[index] == 0x5C, bytes[index + 1] == 0x75,
               let value = hexValue(bytes[(index + 2)..<(index + 6)]) {
                if value == 0xFEFF { bom = true }
                held.insert(value)
            }
            index += 1
        }
        guard bom else { return nil }
        return (UInt32(0xE000)...UInt32(0xF8FF)).lazy.filter { !held.contains($0) }
            .compactMap(Unicode.Scalar.init).first
    }

    /// `bytes` with every U+FEFF inside a string — raw, or a `\uFEFF`
    /// escape — replaced by `stand`, raw or escaped alike. Outside strings
    /// nothing changes (a byte order mark before the document stays one).
    private static func masked(_ bytes: [UInt8], with stand: Unicode.Scalar) -> [UInt8] {
        let raw = Array(String(stand).utf8)
        let escape = Array(String(format: "\\u%04X", stand.value).utf8)
        var out: [UInt8] = []
        out.reserveCapacity(bytes.count)
        var inString = false
        var index = 0
        while index < bytes.count {
            let byte = bytes[index]
            if !inString {
                if byte == 0x22 { inString = true }
                out.append(byte)
                index += 1
            } else if byte == 0x22 {
                inString = false
                out.append(byte)
                index += 1
            } else if byte == 0x5C {
                if index + 5 < bytes.count, bytes[index + 1] == 0x75,
                   hexValue(bytes[(index + 2)..<(index + 6)]) == 0xFEFF {
                    out += escape
                    index += 6
                } else {
                    // Any other escape: the backslash and what it escapes.
                    out += bytes[index..<min(index + 2, bytes.count)]
                    index += 2
                }
            } else if byte == 0xEF, index + 2 < bytes.count,
                      bytes[index + 1] == 0xBB, bytes[index + 2] == 0xBF {
                out += raw
                index += 3
            } else {
                out.append(byte)
                index += 1
            }
        }
        return out
    }

    /// Four hex digits, either case, as a value.
    private static func hexValue(_ digits: ArraySlice<UInt8>) -> UInt32? {
        var value: UInt32 = 0
        for digit in digits {
            switch digit {
            case 0x30...0x39: value = value << 4 | UInt32(digit - 0x30)
            case 0x41...0x46: value = value << 4 | UInt32(digit - 0x41 + 10)
            case 0x61...0x66: value = value << 4 | UInt32(digit - 0x61 + 10)
            default: return nil
            }
        }
        return value
    }

    /// What JSONSerialization made of the masked data, with `stand` back as
    /// U+FEFF in every string and key.
    private static func restored(_ value: Any, _ stand: Unicode.Scalar) -> Any {
        switch value {
        case let string as String:
            return restored(string, stand)
        case let array as [Any]:
            return array.map { restored($0, stand) }
        case let object as [String: Any]:
            var out: [String: Any] = [:]
            for (key, member) in object { out[restored(key, stand)] = restored(member, stand) }
            return out
        default:
            return value
        }
    }

    private static func restored(_ string: String, _ stand: Unicode.Scalar) -> String {
        var scalars = String.UnicodeScalarView()
        for scalar in string.unicodeScalars { scalars.append(scalar == stand ? "\u{FEFF}" : scalar) }
        return String(scalars)
    }
}
