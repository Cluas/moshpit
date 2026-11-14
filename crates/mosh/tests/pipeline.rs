//! `MoshPipelineTests.swift`, one to one: compression, crypto framing,
//! bootstrap parsing and command construction, and the instruction pipeline
//! end to end.

use moshpit_mosh::bootstrap::{self, BootstrapError, EXTRA_PATH_DIRS};
use moshpit_mosh::compress;
use moshpit_mosh::wire::{
    Fragment, FragmentAssembler, Instruction, UserEvent, decode_user_message, encode_user_message,
    fragment,
};
use moshpit_mosh::{Direction, Session};

const KEY_0_15: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
/// `KEY_0_15` as mosh prints it: 22 unpadded base64 characters.
const KEY_0_15_B64: &str = "AAECAwQFBgcICQoLDA0ODw";

// MARK: Compression

/// zlib compress → decompress round-trips arbitrary payloads
#[test]
fn compression_round_trip() {
    let samples: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"hello".to_vec(),
        vec![0x41; 5000],
        (0..=255).collect(),
    ];
    for sample in samples {
        let packed = compress::compress(&sample);
        assert_eq!(packed[0], 0x78, "zlib header byte present");
        assert_eq!(compress::decompress(&packed), Ok(sample));
    }
}

fn hex(text: &str) -> Vec<u8> {
    moshpit_mosh::harness::unhex(text).unwrap()
}

/// Inflation is as strict as zlib's `uncompress` (`MoshPipelineTests`'
/// `decompressionIsStrict`, the same cases).
#[test]
fn decompression_is_strict() {
    use compress::CompressionError::{BadChecksum, BadHeader, InflateFailed, TooLarge};
    let good = compress::compress(b"hello");
    assert_eq!(compress::decompress(&good).unwrap(), b"hello");
    // Bytes after the trailer are ignored, as zlib ignores them.
    let trailing = [good.as_slice(), &[1, 2, 3]].concat();
    assert_eq!(compress::decompress(&trailing).unwrap(), b"hello");

    let mut trailer = good.clone();
    *trailer.last_mut().unwrap() ^= 1;
    assert_eq!(compress::decompress(&trailer), Err(BadChecksum));
    assert_eq!(
        compress::decompress(&good[..good.len() - 1]),
        Err(InflateFailed)
    );
    for header in [[0x78, 0x9D], [0x79, 0x18], [0x88, 0x1C], [0x78, 0xBB]] {
        let bad = [header.as_slice(), &good[2..]].concat();
        assert_eq!(compress::decompress(&bad), Err(BadHeader), "{header:02x?}");
    }
    // A stored block whose NLEN is not ~LEN, and an invalid code-length set.
    assert_eq!(
        compress::decompress(&hex(
            "789c013000cfbfffefc612fec5929a8c0c3cfb3e28ff1d7e18a2919ea37be12928e1e85a8b5d908552fd5b2919877dc420bc53d54f0c9436692617bc"
        )),
        Err(InflateFailed)
    );
    assert_eq!(
        compress::decompress(&hex("789c95c0810000000180207fc37066389e00e09d2881")),
        Err(InflateFailed)
    );
    // mosh's own buffer: exactly 4 MiB inflates, one byte more does not.
    let limit = compress::MAX_DECOMPRESSED_LEN;
    assert_eq!(
        compress::decompress(&compress::compress(&vec![0; limit]))
            .unwrap()
            .len(),
        limit
    );
    assert_eq!(
        compress::decompress(&compress::compress(&vec![0; limit + 1])),
        Err(TooLarge)
    );
}

/// A literal/length or distance code set of a single code is incomplete,
/// and zlib takes it only when that code is one bit long ("invalid
/// distances set", "invalid literal/lengths set"). miniz_oxide, the crate's
/// inflater before, took any length: it accepted these streams, each with
/// a valid Adler-32, which zlib (mosh's `uncompress`, the Swift client)
/// refuses. Made by `harness/equivalence.py`'s `dynamic_block` and checked
/// against the real zlib there.
#[test]
fn a_single_code_set_is_one_bit_or_refused() {
    use compress::CompressionError::InflateFailed;
    // "aaa", the distance set one code of length 1: zlib inflates it.
    assert_eq!(
        compress::decompress(&hex("789c05c08100000000009056ff132002490124")).unwrap(),
        b"aaa"
    );
    // The same with that code 2 and 3 bits long.
    for stream in [
        "789c05c001010000008090adfe9f100202490124",
        "789c05c001010000000290adfe9f100202490124",
    ] {
        assert_eq!(compress::decompress(&hex(stream)), Err(InflateFailed));
    }
    // An empty block whose literal/length set is the end-of-block code
    // alone: one bit long inflates, two or three bits do not.
    assert_eq!(
        compress::decompress(&hex("789c05c0010500000000a0ffaf0300000001")).unwrap(),
        b""
    );
    for stream in [
        "789c0580010500000080febf0e00000001",
        "789c05400105000000faff3a0000000001",
    ] {
        assert_eq!(compress::decompress(&hex(stream)), Err(InflateFailed));
    }
    // A corrupted stream equivalence.py's fuzzing found (distance set: one
    // code of length 3): zlib's verdict, not a checksum mismatch.
    assert_eq!(
        compress::decompress(&hex(
            "789c95c8c10900200805d055fe6c4551088aa2e8f8cee0f53bd0f89bb04c9271a5e0036b0021175b"
        )),
        Err(InflateFailed)
    );
}

/// Every DEFLATE block type and the edges of the decoder: fixed and dynamic
/// codes, long matches that overlap their own output, stored blocks, codes
/// longer than the fast table, and the stream ending inside each.
#[test]
fn inflation_round_trips_and_refuses_truncation() {
    let mut rng = moshpit_mosh::harness::SplitMix64(5);
    let mut samples: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"a".to_vec(),
        vec![b'x'; 100_000],
        b"abcabcabcabc".repeat(500),
        (0..=255u8).cycle().take(70_000).collect(),
    ];
    // Skewed byte frequencies give literal codes longer than nine bits.
    samples.push(
        (0..40_000)
            .map(|_| {
                let r = rng.next_u64();
                if r.is_multiple_of(64) {
                    (r >> 8) as u8
                } else {
                    b'e'
                }
            })
            .collect(),
    );
    samples.push(rng.bytes(20_000));
    for sample in &samples {
        for stream in [
            compress::compress(sample),
            // Stored blocks (miniz level 0), several when long.
            [
                &[0x78, 0x01][..],
                &miniz_oxide::deflate::compress_to_vec(sample, 0),
                &compress::adler32(sample).to_be_bytes(),
            ]
            .concat(),
        ] {
            assert_eq!(&compress::decompress(&stream).unwrap(), sample);
            // Past the 6 bytes under which the header check refuses it.
            for cut in [6, stream.len() / 2, stream.len() - 5] {
                if cut >= 6 && cut < stream.len() - 4 {
                    assert_eq!(
                        compress::decompress(&stream[..cut]),
                        Err(compress::CompressionError::InflateFailed),
                        "{} bytes cut at {cut}",
                        sample.len()
                    );
                }
            }
        }
    }
}

/// Adler-32 matches the RFC 1950 example
#[test]
fn adler32_known() {
    // Adler-32 of "Wikipedia" is 0x11E60398.
    assert_eq!(compress::adler32(b"Wikipedia"), 0x11E6_0398);
}

// MARK: Crypto framing

/// seal → open round-trips a Message and recovers seq/direction
#[test]
fn crypto_round_trip() {
    let crypto = Session::new(&KEY_0_15).unwrap();
    let packet = crypto.seal(42, Direction::ToServer, 0x1234, 0xABCD, b"payload-bytes");
    // Wire nonce is the first 8 bytes, big-endian, top bit = direction (0).
    assert!(packet.len() > 8 + 16);
    let opened = crypto.open(&packet).unwrap();
    assert_eq!(opened.timestamp, 0x1234);
    assert_eq!(opened.timestamp_reply, 0xABCD);
    assert_eq!(opened.payload, b"payload-bytes");
    assert_eq!(opened.seq, 42);
    assert_eq!(opened.direction, Direction::ToServer);
}

/// a tampered datagram fails authentication
#[test]
fn crypto_tamper_rejected() {
    let crypto = Session::new(&[0x5A; 16]).unwrap();
    let mut packet = crypto.seal(7, Direction::ToClient, 1, 2, &[0xDE, 0xAD]);
    *packet.last_mut().unwrap() ^= 0xFF; // flip a tag byte
    assert!(crypto.open(&packet).is_err());
}

// MARK: Bootstrap parsing

/// MOSH CONNECT line parses into host/port/16-byte key
#[test]
fn bootstrap_parse() {
    let output = format!(
        "\r\nMOSH CONNECT 60001 {KEY_0_15_B64}\r\n\r\nmosh-server (mosh 1.4.0) [build mosh 1.4.0]\r\n"
    );
    let creds = bootstrap::parse(&output, "10.0.0.5").unwrap();
    assert_eq!(creds.host, "10.0.0.5");
    assert_eq!(creds.udp_port, 60001);
    assert_eq!(creds.key, KEY_0_15);
}

/// missing MOSH CONNECT line throws
#[test]
fn bootstrap_no_line() {
    assert!(bootstrap::parse("permission denied\n", "h").is_err());
}

/// A well-formed connect line carrying `port`, for the range tests below.
fn connect_line(port: &str) -> String {
    format!("MOSH CONNECT {port} {KEY_0_15_B64}\r\n")
}

/// a port that parses but cannot be a port is an error, not a crash
#[test]
fn bootstrap_rejects_unusable_port() {
    // The out-of-range ones used to reach MoshTransport.init and trap in
    // UInt16(udpPort). "0" cannot connect, so it fails here too.
    for port in ["0", "-1", "65536", "70000", "99999999999"] {
        assert_eq!(
            bootstrap::parse(&connect_line(port), "h"),
            Err(BootstrapError::UnusablePort(port.parse().unwrap())),
            "port {port}"
        );
    }
}

/// the edges of the usable range still parse
#[test]
fn bootstrap_accepts_port_range() {
    for (input, expected) in [("1", 1u16), ("65535", 65535), ("60001", 60001)] {
        let creds = bootstrap::parse(&connect_line(input), "h").unwrap();
        assert_eq!(creds.udp_port, expected);
    }
}

/// the server pid comes only from mosh-server's own detach line
#[test]
fn bootstrap_pid_comes_from_the_detach_line() {
    let output = format!(
        "rapid = 5\n{}\nmosh-server (mosh 1.4.0) [build mosh 1.4.0]\n[mosh-server detached, pid = 77]\n",
        connect_line("60001")
    );
    assert_eq!(bootstrap::parse(&output, "h").unwrap().server_pid, Some(77));
}

/// no pid to kill unless the detach line names one above 1 (pid 0 makes
/// `kill 0` signal the whole process group; 1 is init)
#[test]
fn bootstrap_leaves_nothing_to_kill() {
    for tail in [
        "[mosh-server detached, pid = 0]\n",
        "[mosh-server detached, pid = 1]\n",
        "pid = 4242\n",
        "[mosh-server detached, pid = ]\n",
        "[mosh-server detached, pid = 99999999999999999999]\n",
        "",
    ] {
        let output = connect_line("60001") + tail;
        assert_eq!(
            bootstrap::parse(&output, "h").unwrap().server_pid,
            None,
            "{tail:?}"
        );
    }
}

/// a non-numeric port field is still the missing-line error
#[test]
fn bootstrap_non_numeric_port() {
    assert!(matches!(
        bootstrap::parse(&connect_line("sixty"), "h"),
        Err(BootstrapError::NoConnectLine(_))
    ));
}

// MARK: Bootstrap command construction

/// The probe's PATH extension, prepended for bare binary names.
fn path_prefix() -> String {
    format!("PATH=\"$PATH:{EXTRA_PATH_DIRS}\" ")
}

/// configured UDP port range reaches mosh-server as -p start:end
#[test]
fn bootstrap_command_forwards_port_range() {
    let cmd = bootstrap::command("mosh-server", "en_US.UTF-8", Some(60001), Some(60999));
    assert_eq!(
        cmd,
        path_prefix() + "mosh-server new -s -c 256 -p 60001:60999 -l LANG=en_US.UTF-8"
    );
}

/// no configured range omits -p (mosh-server default range)
#[test]
fn bootstrap_command_default_range() {
    let cmd = bootstrap::command("mosh-server", "en_US.UTF-8", None, None);
    assert_eq!(
        cmd,
        path_prefix() + "mosh-server new -s -c 256 -l LANG=en_US.UTF-8"
    );
}

/// explicit server path is trusted verbatim — no PATH prefix
#[test]
fn bootstrap_command_explicit_path_unprefixed() {
    let cmd = bootstrap::command("/opt/homebrew/bin/mosh-server", "en_US.UTF-8", None, None);
    assert_eq!(
        cmd,
        "/opt/homebrew/bin/mosh-server new -s -c 256 -l LANG=en_US.UTF-8"
    );
}

/// degenerate ranges degrade instead of erroring remotely
#[test]
fn bootstrap_command_degenerate_ranges() {
    // Inverted / equal end → single-port form.
    assert_eq!(
        bootstrap::command("m", "L", Some(60050), Some(60001)),
        path_prefix() + "m new -s -c 256 -p 60050 -l LANG=L"
    );
    // Out-of-range start → no -p at all.
    assert_eq!(
        bootstrap::command("m", "L", Some(0), Some(61000)),
        path_prefix() + "m new -s -c 256 -l LANG=L"
    );
    assert_eq!(
        bootstrap::command("m", "L", Some(70000), Some(71000)),
        path_prefix() + "m new -s -c 256 -l LANG=L"
    );
}

/// noConnectLine error escapes control chars and caps excerpt length
#[test]
fn bootstrap_error_sanitizes_output() {
    // A hostile remote embeds an ANSI escape (ESC[2J clears the screen) plus
    // a huge junk payload. Neither should survive into the error string.
    let hostile = format!("\u{1B}[2J\u{07}oops{}", "A".repeat(500));
    let message = BootstrapError::NoConnectLine(hostile).to_string();

    // No raw control characters leak through.
    assert!(!message.chars().any(char::is_control));
    // The ESC and BEL are rendered as visible escapes instead.
    assert!(message.contains("\\u{001B}"));
    assert!(message.contains("\\u{0007}"));
    // Ordinary text is preserved.
    assert!(message.contains("oops"));
    // The 500-char junk tail is truncated well under the raw length.
    assert!(message.contains('…'));
    assert!(message.chars().count() < 200);
}

// MARK: Fragment + instruction end-to-end (compressed)

/// instruction → compress → fragment → reassemble → decompress → parse
#[test]
fn transport_instruction_pipeline() {
    let instruction = Instruction {
        old_num: 3,
        new_num: 9,
        ack_num: 5,
        diff: encode_user_message(&[UserEvent::Keystroke(b"ls -la\r".to_vec())]),
        ..Instruction::default()
    };

    let compressed = compress::compress(&instruction.encode());
    let mut assembler = FragmentAssembler::new();
    let mut reassembled = None;
    for piece in fragment(&compressed, 1, 1300) {
        let parsed = Fragment::parse(&piece.encode()).unwrap();
        if let Some(done) = assembler.receive(parsed) {
            reassembled = Some(done);
        }
    }

    let restored =
        Instruction::decode(&compress::decompress(&reassembled.unwrap()).unwrap()).unwrap();
    assert_eq!(restored.old_num, 3);
    assert_eq!(restored.new_num, 9);
    assert_eq!(restored.ack_num, 5);
    assert_eq!(
        decode_user_message(&restored.diff),
        Ok(vec![UserEvent::Keystroke(b"ls -la\r".to_vec())])
    );
}
