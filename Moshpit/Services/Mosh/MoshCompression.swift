import Foundation
import Compression
import zlib

/// zlib-format (RFC 1950) deflate/inflate matching mosh's `Compressor`,
/// which wraps payloads with zlib's `compress2`/`uncompress` (2-byte header
/// + raw DEFLATE + 4-byte big-endian Adler-32 trailer).
///
/// Apple's `Compression` framework only does raw DEFLATE (RFC 1951), so
/// `compress` adds the zlib header and Adler-32 itself. `decompress` uses
/// the system zlib — the library mosh itself inflates with.
enum MoshCompression {

    enum CompressionError: Error {
        case inflateFailed
        case deflateFailed
        /// Under 6 bytes, or a header zlib's `uncompress` rejects.
        case badHeader
        /// The Adler-32 trailer does not match the inflated bytes.
        case badChecksum
        /// The payload inflates past `maxDecompressedSize`.
        case tooLarge
    }

    /// mosh's `Compressor::BUFFER_SIZE`: zlib's `uncompress` there refuses
    /// any instruction that inflates to more, so no mosh peer sends one.
    static let maxDecompressedSize = 2048 * 2048

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

    /// Inflate a zlib stream the way zlib's `uncompress` does (RFC 1950
    /// requires the checks): the header's check bits, method, window and no
    /// preset dictionary; the DEFLATE data; the Adler-32 that follows it;
    /// mosh's 4 MiB limit. Bytes after the trailer are ignored, as zlib
    /// ignores them.
    ///
    /// This used to strip two header and four trailer bytes unchecked and
    /// raw-inflate what lay between with Apple's decoder, which also lets a
    /// stored block whose NLEN is not ~LEN and an invalid code-length set
    /// through. Datagrams are authenticated, so only a broken server could
    /// send such a stream.
    static func decompress(_ data: Data) throws -> Data {
        let input = [UInt8](data)
        guard input.count >= 6, headerIsValid(cmf: input[0], flg: input[1]) else {
            throw CompressionError.badHeader
        }
        var stream = z_stream()
        // -15: raw DEFLATE with a 32 KiB window; the header is checked above.
        guard inflateInit2_(&stream, -MAX_WBITS, ZLIB_VERSION, Int32(MemoryLayout<z_stream>.size)) == Z_OK else {
            throw CompressionError.inflateFailed
        }
        defer { inflateEnd(&stream) }

        var output = Data()
        let chunkSize = 32_768
        var chunk = [UInt8](repeating: 0, count: chunkSize)
        let trailer: ArraySlice<UInt8> = try input.withUnsafeBufferPointer { source in
            stream.next_in = UnsafeMutablePointer(mutating: source.baseAddress! + 2)
            stream.avail_in = uInt(source.count - 2)
            while true {
                let status = chunk.withUnsafeMutableBufferPointer { out -> Int32 in
                    stream.next_out = out.baseAddress
                    stream.avail_out = uInt(chunkSize)
                    return inflate(&stream, Z_NO_FLUSH)
                }
                let produced = chunkSize - Int(stream.avail_out)
                guard output.count + produced <= maxDecompressedSize else { throw CompressionError.tooLarge }
                output.append(contentsOf: chunk[..<produced])
                switch status {
                case Z_STREAM_END:
                    let end = source.count - Int(stream.avail_in)
                    return input[end ..< min(end + 4, input.count)]
                case Z_OK:
                    continue
                default:
                    // Z_BUF_ERROR here means the input ran out first: a
                    // truncated stream. Anything else is invalid DEFLATE.
                    throw CompressionError.inflateFailed
                }
            }
        }
        guard trailer.count == 4 else { throw CompressionError.inflateFailed }
        let expected = trailer.reduce(UInt32(0)) { $0 << 8 | UInt32($1) }
        guard adler32(output) == expected else { throw CompressionError.badChecksum }
        return output
    }

    /// zlib's `inflate` header checks: FCHECK (the 16-bit header is a
    /// multiple of 31), CM = 8 (DEFLATE), CINFO <= 7 (a window of at most
    /// 32 KiB), no FDICT (mosh never sets a preset dictionary).
    private static func headerIsValid(cmf: UInt8, flg: UInt8) -> Bool {
        (UInt16(cmf) << 8 | UInt16(flg)) % 31 == 0
            && cmf & 0x0F == 8
            && cmf >> 4 <= 7
            && flg & 0x20 == 0
    }

    // MARK: - Raw DEFLATE via Compression framework

    private static func rawDeflate(_ data: Data) throws -> Data {
        try perform(operation: COMPRESSION_STREAM_ENCODE, input: data)
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
