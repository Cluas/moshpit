//! The system CSPRNG — the only source of pairing secrets, IVs and nonces.

/// Fill `buf` from the system CSPRNG.
///
/// # Panics
///
/// When the system has no randomness to give. A CSPRNG failure is not
/// something to paper over with a fallback of lesser randomness — that is how
/// a "secret" silently becomes guessable (`PushPairing.randomHex` traps the
/// same way).
pub fn fill(buf: &mut [u8]) {
    getrandom::fill(buf).expect("the system CSPRNG failed");
}

/// `count` random bytes as lowercase hex (`PushPairing.randomHex(bytes:)`).
pub fn hex(count: usize) -> String {
    let mut bytes = vec![0u8; count];
    fill(&mut bytes);
    crate::foundation::hex::encode(&bytes)
}
