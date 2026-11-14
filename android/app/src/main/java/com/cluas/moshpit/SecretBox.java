package com.cluas.moshpit;

import java.nio.ByteBuffer;
import java.security.GeneralSecurityException;

import javax.crypto.Cipher;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/**
 * One keychain item sealed with AES-256-GCM: {@code version (1) || iv (12) || ciphertext and tag}. The key is the
 * Android Keystore's ({@link Secrets}), so the plain secret exists only in memory; the item's account name is the
 * additional authenticated data, so an item copied under another account does not open.
 * <p>
 * Plain Java (no Android API): the unit tests seal and open with a software key.
 */
final class SecretBox {
    static final byte VERSION = 1;
    static final int IV_BYTES = 12;
    static final int TAG_BITS = 128;

    private SecretBox() {}

    /** {@code secret} sealed with {@code key} for {@code account}. */
    static byte[] seal(SecretKey key, String account, byte[] secret) throws GeneralSecurityException {
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        // The cipher picks a random IV: the Keystore refuses one from the caller.
        cipher.init(Cipher.ENCRYPT_MODE, key);
        byte[] iv = cipher.getIV();
        if (iv == null || iv.length != IV_BYTES) throw new GeneralSecurityException("no 12-byte IV");
        cipher.updateAAD(account.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        byte[] sealed = cipher.doFinal(secret);
        return ByteBuffer.allocate(1 + IV_BYTES + sealed.length).put(VERSION).put(iv).put(sealed).array();
    }

    /** The secret {@code box} seals for {@code account}; throws when it is not one, or not this key's. */
    static byte[] open(SecretKey key, String account, byte[] box) throws GeneralSecurityException {
        if (box == null || box.length < 1 + IV_BYTES + TAG_BITS / 8 || box[0] != VERSION) {
            throw new GeneralSecurityException("not a sealed item");
        }
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.DECRYPT_MODE, key, new GCMParameterSpec(TAG_BITS, box, 1, IV_BYTES));
        cipher.updateAAD(account.getBytes(java.nio.charset.StandardCharsets.UTF_8));
        return cipher.doFinal(box, 1 + IV_BYTES, box.length - 1 - IV_BYTES);
    }
}
