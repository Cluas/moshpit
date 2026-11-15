//! `MoshWireTests.swift`, one to one: golden bytes for the instruction and
//! the user events, round trips, unknown fields, the fragment header,
//! fragmenting, reassembly and the host message's echo ack.

use moshpit_mosh::harness::hex;
use moshpit_mosh::proto::ProtoWriter;
use moshpit_mosh::wire::{
    Fragment, FragmentAssembler, HostMessage, Instruction, UserEvent, WireError,
    decode_user_message, encode_user_message, fragment,
};

/// TransportInstruction encodes to known proto2 bytes
#[test]
fn transport_instruction_golden_bytes() {
    let instruction = Instruction {
        protocol_version: 2,
        old_num: 1,
        new_num: 2,
        ack_num: 3,
        throwaway_num: 0,
        diff: b"hi".to_vec(),
        chaff: Vec::new(),
    };
    // 08 02 | 10 01 | 18 02 | 20 03 | 28 00 | 32 02 68 69
    assert_eq!(hex(&instruction.encode()), "0802100118022003280032026869");
}

/// TransportInstruction round-trips, including multi-byte varints
#[test]
fn transport_instruction_round_trip() {
    let instruction = Instruction {
        protocol_version: u32::MAX,
        old_num: u64::MAX,
        new_num: 1 << 40,
        ack_num: 12_345,
        throwaway_num: 7,
        diff: vec![0x00, 0xFF, 0x01],
        chaff: vec![0x09, 0x09],
    };
    assert_eq!(Instruction::decode(&instruction.encode()), Ok(instruction));
}

/// TransportInstruction decode skips unknown fields
#[test]
fn transport_instruction_skips_unknown_fields() {
    let mut writer = ProtoWriter::new();
    writer.field_varint(3, 9); // new_num
    writer.field_varint(12, 999); // unknown varint
    writer.field_bytes(13, &[1, 2, 3]); // unknown length-delimited
    writer.field_varint(4, 8); // ack_num
    let decoded = Instruction::decode(writer.bytes()).unwrap();
    assert_eq!(decoded.new_num, 9);
    assert_eq!(decoded.ack_num, 8);
}

/// decode throws on truncated input
#[test]
fn transport_instruction_truncated() {
    // Key for field 6 (bytes) declaring 200 bytes of payload, but none present.
    assert_eq!(
        Instruction::decode(&[0x32, 0xC8, 0x01]),
        Err(WireError::TruncatedField)
    );
}

/// keystroke encodes as Instruction ext field 2 with keys at field 4
#[test]
fn keystroke_golden_bytes() {
    let message = encode_user_message(&[UserEvent::Keystroke(b"a".to_vec())]);
    // UserMessage: f1 LEN { Instruction: f2 LEN { Keystroke: f4 LEN "a" } }
    assert_eq!(hex(&message), "0a051203220161");
}

/// resize encodes as Instruction ext field 3 with width=5, height=6
#[test]
fn resize_golden_bytes() {
    let message = encode_user_message(&[UserEvent::Resize {
        width: 80,
        height: 24,
    }]);
    // f1 LEN { f3 LEN { f5 varint 80, f6 varint 24 } }
    assert_eq!(hex(&message), "0a061a0428503018");
}

/// UserMessage round-trips mixed keystrokes and resizes
#[test]
fn user_message_round_trip() {
    let events = vec![
        UserEvent::Keystroke(b"ls -la\n".to_vec()),
        UserEvent::Resize {
            width: 132,
            height: 43,
        },
        UserEvent::Keystroke(vec![0x03]), // ^C
    ];
    assert_eq!(
        decode_user_message(&encode_user_message(&events)),
        Ok(events)
    );
}

/// UserMessage decode ignores instructions with unknown extensions
/// `UserEvent::encoded_len` is what the event adds to a `UserMessage`: the
/// transport sizes its instructions with it.
#[test]
fn user_event_encoded_len_matches_the_encoder() {
    let mut events: Vec<UserEvent> = [
        0, 1, 2, 120, 121, 127, 128, 16_370, 16_383, 16_384, 65_504, 65_536,
    ]
    .into_iter()
    .chain([2_097_151, 2_097_152])
    .map(|len| UserEvent::Keystroke(vec![b'k'; len]))
    .collect();
    for (width, height) in [
        (80, 24),
        (127, 128),
        (16_384, 1),
        (-1, -1),
        (i32::MAX, i32::MIN),
    ] {
        events.push(UserEvent::Resize { width, height });
    }
    for event in &events {
        assert_eq!(
            event.encoded_len(),
            encode_user_message(std::slice::from_ref(event)).len(),
            "{:?}",
            match event {
                UserEvent::Keystroke(keys) => format!("{} keys", keys.len()),
                resize => format!("{resize:?}"),
            }
        );
    }
    let total: usize = events.iter().map(UserEvent::encoded_len).sum();
    assert_eq!(total, encode_user_message(&events).len());
}

#[test]
fn user_message_ignores_unknown_extensions() {
    let mut unknown_instruction = ProtoWriter::new();
    unknown_instruction.field_bytes(99, &[0x01]); // future extension
    let mut writer = ProtoWriter::new();
    writer.field_bytes(1, unknown_instruction.bytes());
    let mut keystroke = ProtoWriter::new();
    keystroke.field_bytes(4, b"x"); // Keystroke.keys = field 4
    let mut instruction = ProtoWriter::new();
    instruction.field_bytes(2, keystroke.bytes()); // Keystroke = ext field 2
    writer.field_bytes(1, instruction.bytes());

    assert_eq!(
        decode_user_message(writer.bytes()),
        Ok(vec![UserEvent::Keystroke(b"x".to_vec())])
    );
}

/// fragment header is 8-byte BE id plus 2-byte BE num with final bit
#[test]
fn fragment_golden_header() {
    let fragment = Fragment::new(0x0102030405060708, 1, true, Vec::new());
    assert_eq!(hex(&fragment.encode()), "01020304050607088001");
}

/// fragment encode/parse round-trips
#[test]
fn fragment_round_trip() {
    let fragment = Fragment::new(u64::MAX, 0x7FFF, false, vec![1, 2, 3]);
    assert_eq!(Fragment::parse(&fragment.encode()), Ok(fragment));
}

/// parsing rejects datagrams shorter than the 10-byte header
#[test]
fn fragment_too_short() {
    assert_eq!(Fragment::parse(&[0; 9]), Err(WireError::TruncatedFragment));
}

/// fragmenting splits payloads at mtu minus header and marks the last final
#[test]
fn fragmenting() {
    let payload: Vec<u8> = (0..2500).map(|i| (i % 251) as u8).collect();
    let fragments = fragment(&payload, 42, 1280);
    assert_eq!(fragments.len(), 2);
    assert_eq!(fragments[0].contents.len(), 1270);
    assert!(!fragments[0].is_final);
    assert!(fragments[1].is_final);
    assert_eq!(
        fragments.iter().map(|f| f.num).collect::<Vec<_>>(),
        vec![0, 1]
    );
}

/// empty payload still produces one final fragment
#[test]
fn fragmenting_empty_payload() {
    let fragments = fragment(&[], 7, 1280);
    assert_eq!(fragments.len(), 1);
    assert!(fragments[0].is_final);
    assert!(fragments[0].contents.is_empty());
}

/// assembler rebuilds the payload from in-order fragments
#[test]
fn assembler_rebuilds_payload() {
    let payload: Vec<u8> = (0..5000).map(|i| (i % 256) as u8).collect();
    let fragments = fragment(&payload, 9, 1280);
    let mut assembler = FragmentAssembler::new();
    let mut result = None;
    for piece in fragments {
        assert_eq!(result, None);
        result = assembler.receive(Fragment::parse(&piece.encode()).unwrap());
    }
    assert_eq!(result, Some(payload));
}

/// assembler drops fragments from stale instruction ids
#[test]
fn assembler_drops_stale_ids() {
    let mut assembler = FragmentAssembler::new();
    let newer = assembler.receive(Fragment::new(5, 0, true, vec![1]));
    assert_eq!(newer, Some(vec![1]));
    let stale = assembler.receive(Fragment::new(4, 0, true, vec![2]));
    assert_eq!(stale, None);
}

/// assembler resets partial state when a newer instruction arrives
#[test]
fn assembler_resets_on_newer_id() {
    let mut assembler = FragmentAssembler::new();
    assert_eq!(assembler.receive(Fragment::new(1, 0, false, vec![1])), None);
    // Newer instruction supersedes the unfinished one.
    assert_eq!(
        assembler.receive(Fragment::new(2, 0, true, vec![9])),
        Some(vec![9])
    );
}

/// HostMessage reads the echo ack (Instruction field 7, echo_ack_num field 8)
#[test]
fn host_message_echo_ack() {
    let mut ack = ProtoWriter::new();
    ack.field_varint(8, 1 << 33);
    let mut instruction = ProtoWriter::new();
    instruction.field_bytes(7, ack.bytes());
    let mut message = ProtoWriter::new();
    message.field_bytes(1, instruction.bytes());

    let parsed = HostMessage::decode(message.bytes()).unwrap();
    assert_eq!(parsed.echo_ack, Some(1 << 33));
    assert!(parsed.host_bytes.is_empty());

    let mut host_bytes = ProtoWriter::new();
    host_bytes.field_bytes(4, b"x");
    let mut output = ProtoWriter::new();
    output.field_bytes(2, host_bytes.bytes());
    let mut bare = ProtoWriter::new();
    bare.field_bytes(1, output.bytes());
    assert_eq!(HostMessage::decode(bare.bytes()).unwrap().echo_ack, None);
}

/// Fix (fragment-count): one byte a fragment, the 65,536th piece no longer
/// overflows the counter (it trapped the Swift client); the number wraps in
/// 15 bits and only the last is final. Pinned by MoshWireTests until the
/// Swift client went (E2).
#[test]
fn fragmenting_past_the_counter_does_not_trap() {
    let fragments = fragment(&vec![0u8; 65_536], 1, 11);
    assert_eq!(fragments.len(), 65_536);
    assert!(fragments.last().is_some_and(|f| f.is_final));
    assert_eq!(fragments.iter().filter(|f| f.is_final).count(), 1);
    assert_eq!(fragments[32_767].num, 0x7FFF);
    assert_eq!(fragments[32_768].num, 0);
    assert_eq!(fragments.last().map(|f| f.num), Some(0x7FFF));
}
