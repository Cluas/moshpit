import Foundation
import Compression

/// zlib-format (RFC 1950) deflate/inflate matching mosh's `Compressor`,
/// which wraps payloads with zlib's `compress2`/`uncompress` (2-byte header
/// + raw DEFLATE + 4-byte big-endian Adler-32 trailer).
///
/// Apple's `Compression` framework only does raw DEFLATE (RFC 1951), so we
/// add/strip the zlib header and Adler-32 ourselves.
enum MoshCompression {

    enum CompressionError: Error {
        case inflateFailed
        case deflateFailed
        case badHeader
    }

    /// zlib-wrap raw DEFLATE of `data`. Header `0x78 0x9C` = 32 KB window,
    /// default compression — exactly what `compress2(..., Z_DEFAULT_COMPRESSION)`
    /// emits, so the mosh server's zlib accepts it.
    static func compress(_ data: Data) throws -> Data {
        let raw = try rawDeflate(data)
        var out = Data([0x78, 0x9C])
        out.append(raw)
        var adler = adler32(data).bigEndian
        withUnsafeBytes(of: &adler) { out.append(contentsOf: $0) }
        return out
    }

    /// Strip the zlib header + Adler-32 trailer and raw-inflate the middle.
    static func decompress(_ data: Data) throws -> Data {
        guard data.count >= 6 else { throw CompressionError.badHeader }
        // Bytes 0..2 = zlib header, last 4 = Adler-32. Everything between is
        // the raw DEFLATE stream.
        let raw = data.subdata(in: 2 ..< data.count - 4)
        return try rawInflate(raw)
    }

    // MARK: - Raw DEFLATE via Compression framework

    private static func rawDeflate(_ data: Data) throws -> Data {
        try perform(operation: COMPRESSION_STREAM_ENCODE, input: data)
    }

    private static func rawInflate(_ data: Data) throws -> Data {
        try perform(operation: COMPRESSION_STREAM_DECODE, input: data)
    }

    private static func perform(operation: compression_stream_operation, input: Data) throws -> Data {
        var stream = compression_stream(dst_ptr: UnsafeMutablePointer<UInt8>(bitPattern: 1)!,
                                        dst_size: 0,
                                        src_ptr: UnsafeMutablePointer<UInt8>(bitPattern: 1)!,
                                        src_size: 0,
                                        state: nil)
        guard compression_stream_init(&stream, operation, COMPRESSION_ZLIB) == COMPRESSION_STATUS_OK else {
            throw operation == COMPRESSION_STREAM_ENCODE ? CompressionError.deflateFailed : CompressionError.inflateFailed
        }
        defer { compression_stream_destroy(&stream) }

        let bufferSize = 32_768
        let dst = UnsafeMutablePointer<UInt8>.allocate(capacity: bufferSize)
        defer { dst.deallocate() }

        var output = Data()
        let inputBytes = [UInt8](input)

        return try inputBytes.withUnsafeBufferPointer { srcBuf -> Data in
            stream.src_ptr = srcBuf.baseAddress ?? UnsafePointer<UInt8>(bitPattern: 1)!
            stream.src_size = srcBuf.count
            let flags = Int32(COMPRESSION_STREAM_FINALIZE.rawValue)

            while true {
                stream.dst_ptr = dst
                stream.dst_size = bufferSize
                let status = compression_stream_process(&stream, flags)
                switch status {
                case COMPRESSION_STATUS_OK, COMPRESSION_STATUS_END:
                    let produced = bufferSize - stream.dst_size
                    if produced > 0 { output.append(dst, count: produced) }
                    if status == COMPRESSION_STATUS_END { return output }
                default:
                    throw operation == COMPRESSION_STREAM_ENCODE ? CompressionError.deflateFailed : CompressionError.inflateFailed
                }
            }
        }
    }

    // MARK: - Adler-32 (RFC 1950)

    static func adler32(_ data: Data) -> UInt32 {
        let modAdler: UInt32 = 65_521
        var a: UInt32 = 1, b: UInt32 = 0
        for byte in data {
            a = (a + UInt32(byte)) % modAdler
            b = (b + a) % modAdler
        }
        return (b << 16) | a
    }
}
