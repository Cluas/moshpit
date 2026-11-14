//! `OCB3Tests.swift`, one to one: RFC 7253 Appendix A vectors through seal,
//! open and a tampered tag, the RFC's iteration test, the mosh usage shape,
//! and input validation. The Swift titles are the doc comments.

use moshpit_mosh::harness::{hex, unhex};
use moshpit_mosh::{Ocb3, OcbError};

/// RFC 7253 Appendix A: (nonce, associated data, plaintext, ciphertext||tag),
/// K = 000102030405060708090A0B0C0D0E0F, 128-bit tag.
const RFC_VECTORS: &[(&str, &str, &str, &str)] = &[
    (
        "BBAA99887766554433221100",
        "",
        "",
        "785407BFFFC8AD9EDCC5520AC9111EE6",
    ),
    (
        "BBAA99887766554433221101",
        "0001020304050607",
        "0001020304050607",
        "6820B3657B6F615A5725BDA0D3B4EB3A257C9AF1F8F03009",
    ),
    (
        "BBAA99887766554433221102",
        "0001020304050607",
        "",
        "81017F8203F081277152FADE694A0A00",
    ),
    (
        "BBAA99887766554433221103",
        "",
        "0001020304050607",
        "45DD69F8F5AAE72414054CD1F35D82760B2CD00D2F99BFA9",
    ),
    (
        "BBAA99887766554433221104",
        "000102030405060708090A0B0C0D0E0F",
        "000102030405060708090A0B0C0D0E0F",
        "571D535B60B277188BE5147170A9A22C3AD7A4FF3835B8C5701C1CCEC8FC3358",
    ),
    (
        "BBAA99887766554433221105",
        "000102030405060708090A0B0C0D0E0F",
        "",
        "8CF761B6902EF764462AD86498CA6B97",
    ),
    (
        "BBAA99887766554433221106",
        "",
        "000102030405060708090A0B0C0D0E0F",
        "5CE88EC2E0692706A915C00AEB8B2396F40E1C743F52436BDF06D8FA1ECA343D",
    ),
    (
        "BBAA99887766554433221107",
        "000102030405060708090A0B0C0D0E0F1011121314151617",
        "000102030405060708090A0B0C0D0E0F1011121314151617",
        "1CA2207308C87C010756104D8840CE1952F09673A448A122C92C62241051F57356D7F3C90BB0E07F",
    ),
    (
        "BBAA99887766554433221108",
        "000102030405060708090A0B0C0D0E0F1011121314151617",
        "",
        "6DC225A071FC1B9F7C69F93B0F1E10DE",
    ),
    (
        "BBAA99887766554433221109",
        "",
        "000102030405060708090A0B0C0D0E0F1011121314151617",
        "221BD0DE7FA6FE993ECCD769460A0AF2D6CDED0C395B1C3CE725F32494B9F914D85C0B1EB38357FF",
    ),
    (
        "BBAA9988776655443322110A",
        "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
        "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
        "BD6F6C496201C69296C11EFD138A467ABD3C707924B964DEAFFC40319AF5A48540FBBA186C5553C68AD9F592A79A4240",
    ),
    (
        "BBAA9988776655443322110B",
        "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
        "",
        "FE80690BEE8A485D11F32965BC9D2A32",
    ),
    (
        "BBAA9988776655443322110C",
        "",
        "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F",
        "2942BFC773BDA23CABC6ACFD9BFD5835BD300F0973792EF46040C53F1432BCDFB5E1DDE3BC18A5F840B52E653444D5DF",
    ),
    (
        "BBAA9988776655443322110D",
        "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F2021222324252627",
        "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F2021222324252627",
        "D5CA91748410C1751FF8A2F618255B68A0A12E093FF454606E59F9C1D0DDC54B65E8628E568BAD7AED07BA06A4A69483A7035490C5769E60",
    ),
    (
        "BBAA9988776655443322110E",
        "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F2021222324252627",
        "",
        "C5CD9D1850C141E358649994EE701B68",
    ),
    (
        "BBAA9988776655443322110F",
        "",
        "000102030405060708090A0B0C0D0E0F101112131415161718191A1B1C1D1E1F2021222324252627",
        "4412923493C57D5DE0D700F753CCE0D1D2D95060122E9F15A5DDBFC5787E50B5CC55EE507BCB084E479AD363AC366B95A98CA5F3000B1479",
    ),
];

fn bytes(text: &str) -> Vec<u8> {
    unhex(text).expect("hex")
}

fn rfc_ocb() -> Ocb3 {
    Ocb3::new(&bytes("000102030405060708090A0B0C0D0E0F")).unwrap()
}

/// seal matches RFC 7253 Appendix A ciphertext
#[test]
fn seal_matches_rfc_vector() {
    let ocb = rfc_ocb();
    for (index, &(n, a, p, c)) in RFC_VECTORS.iter().enumerate() {
        let sealed = ocb.seal(&bytes(p), &bytes(n), &bytes(a)).unwrap();
        assert_eq!(hex(&sealed), c.to_lowercase(), "vector {index}");
    }
}

/// open recovers RFC 7253 Appendix A plaintext
#[test]
fn open_matches_rfc_vector() {
    let ocb = rfc_ocb();
    for (index, &(n, a, p, c)) in RFC_VECTORS.iter().enumerate() {
        let opened = ocb.open(&bytes(c), &bytes(n), &bytes(a)).unwrap();
        assert_eq!(hex(&opened), p.to_lowercase(), "vector {index}");
    }
}

/// open rejects a tampered tag
#[test]
fn open_rejects_tampered_ciphertext() {
    let ocb = rfc_ocb();
    for (index, &(n, a, _, c)) in RFC_VECTORS.iter().enumerate() {
        let mut bad = bytes(c);
        *bad.last_mut().unwrap() ^= 0x01;
        assert_eq!(
            ocb.open(&bad, &bytes(n), &bytes(a)),
            Err(OcbError::AuthenticationFailed),
            "vector {index}"
        );
    }
}

/// RFC 7253 'wider variety of inputs' iteration test
#[test]
fn rfc_iteration_test() {
    // K = zeros(KEYLEN-8) || num2str(TAGLEN, 8); KEYLEN 128, TAGLEN 128.
    let mut key = vec![0u8; 15];
    key.push(128);
    let ocb = Ocb3::new(&key).unwrap();
    let nonce = |n: u64| {
        let mut data = vec![0u8; 4];
        data.extend_from_slice(&n.to_be_bytes());
        data
    };
    let mut accumulated = Vec::new();
    for i in 0..128u64 {
        let s = vec![0u8; i as usize];
        accumulated.extend(ocb.seal(&s, &nonce(3 * i + 1), &s).unwrap());
        accumulated.extend(ocb.seal(&s, &nonce(3 * i + 2), &[]).unwrap());
        accumulated.extend(ocb.seal(&[], &nonce(3 * i + 3), &s).unwrap());
    }
    assert_eq!(accumulated.len(), 22_400);
    let output = ocb.seal(&[], &nonce(385), &accumulated).unwrap();
    assert_eq!(hex(&output), "67e944d23256c5e0b6c61fa22fdf1ea2");
}

/// seal/open round-trip with empty associated data, the mosh configuration
#[test]
fn mosh_shape_round_trip() {
    let ocb = Ocb3::new(&[0xA5; 16]).unwrap();
    let nonce = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let payload: Vec<u8> = (0..1400).map(|i| (i % 256) as u8).collect();
    let sealed = ocb.seal(&payload, &nonce, &[]).unwrap();
    assert_eq!(sealed.len(), payload.len() + 16);
    assert_eq!(ocb.open(&sealed, &nonce, &[]).unwrap(), payload);
}

/// open with the wrong nonce fails authentication
#[test]
fn wrong_nonce_fails() {
    let ocb = Ocb3::new(&[0x42; 16]).unwrap();
    let sealed = ocb.seal(b"hello", &[0; 12], &[]).unwrap();
    let mut other = [0u8; 12];
    other[11] = 1;
    assert_eq!(
        ocb.open(&sealed, &other, &[]),
        Err(OcbError::AuthenticationFailed)
    );
}

/// rejects keys that are not 16 bytes
#[test]
fn rejects_bad_key_length() {
    assert_eq!(
        Ocb3::new(&[0; 32]).map(drop),
        Err(OcbError::InvalidKeyLength)
    );
}

/// rejects nonces that are not 12 bytes
#[test]
fn rejects_bad_nonce_length() {
    let ocb = Ocb3::new(&[0; 16]).unwrap();
    assert_eq!(
        ocb.seal(&[], &[0; 8], &[]),
        Err(OcbError::InvalidNonceLength)
    );
}

/// rejects ciphertext shorter than the tag
#[test]
fn rejects_short_ciphertext() {
    let ocb = Ocb3::new(&[0; 16]).unwrap();
    assert_eq!(
        ocb.open(&[0; 15], &[0; 12], &[]),
        Err(OcbError::CiphertextTooShort)
    );
}
