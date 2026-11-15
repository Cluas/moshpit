import Foundation

/// A host file held open over SFTP (``HostConnection/openRemoteFile(_:)``):
/// what streaming playback pulls byte ranges through. Reads may overlap —
/// each is its own request on the wire. Closed by whoever opened it, or with
/// the connection it came from.
///
/// The Rust session's own SFTP, lent to the app (``BorrowedHost``, W14b),
/// stands behind it.
final class RemoteFileHandle: @unchecked Sendable {
    let size: UInt64
    private let reader: @Sendable (UInt64, UInt32) async throws -> [UInt8]
    private let closer: @Sendable () async -> Void

    /// A file `size` bytes long, read through `read(offset, length)` and
    /// let go with `close`.
    init(size: UInt64,
         read: @escaping @Sendable (UInt64, UInt32) async throws -> [UInt8],
         close: @escaping @Sendable () async -> Void) {
        self.size = size
        self.reader = read
        self.closer = close
    }

    /// At most `length` bytes from `offset` (OpenSSH answers 255 KiB at the
    /// most); empty at the end of the file.
    func read(from offset: UInt64, length: UInt32) async throws -> [UInt8] {
        try await reader(offset, length)
    }

    func close() async {
        await closer()
    }
}
