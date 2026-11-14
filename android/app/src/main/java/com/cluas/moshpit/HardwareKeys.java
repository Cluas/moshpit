package com.cluas.moshpit;

import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;

import java.security.KeyPair;
import java.security.KeyPairGenerator;
import java.security.KeyStore;
import java.security.PrivateKey;
import java.security.Signature;
import java.security.spec.ECGenParameterSpec;

/**
 * SSH keys whose private half never leaves the Android Keystore (the Secure Enclave's part on iOS): ECDSA P-256,
 * made here, signing SHA-256 digests here. The Rust core gets the public half (X.509 SubjectPublicKeyInfo, which
 * {@code moshpit_ssh::keyfile::hardware_public_key} turns into the key's {@code authorized_keys} line) and asks for
 * each signature during a sign-in (ASN.1 DER, which crates/ssh checks against the public half before sending).
 * <p>
 * The key is hardware-backed where the device has a TEE or StrongBox; the emulator's is software. Biometric gating
 * ({@code setUserAuthenticationRequired}) needs a lock screen, so it is not asked for yet.
 */
final class HardwareKeys {
    private HardwareKeys() {}

    private static KeyStore store() throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        return store;
    }

    /** Makes key {@code alias}; its public half, X.509 SubjectPublicKeyInfo. */
    static byte[] generate(String alias) throws Exception {
        KeyPairGenerator generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, "AndroidKeyStore");
        generator.initialize(new KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
                .setAlgorithmParameterSpec(new ECGenParameterSpec("secp256r1"))
                .setDigests(KeyProperties.DIGEST_SHA256)
                .build());
        KeyPair pair = generator.generateKeyPair();
        return pair.getPublic().getEncoded();
    }

    /** {@code data} signed by key {@code alias} with SHA256withECDSA, ASN.1 DER. */
    static byte[] sign(String alias, byte[] data) throws Exception {
        KeyStore.Entry entry = store().getEntry(alias, null);
        if (!(entry instanceof KeyStore.PrivateKeyEntry)) throw new IllegalStateException("no key " + alias);
        PrivateKey key = ((KeyStore.PrivateKeyEntry) entry).getPrivateKey();
        Signature signature = Signature.getInstance("SHA256withECDSA");
        signature.initSign(key);
        signature.update(data);
        return signature.sign();
    }

    static void delete(String alias) {
        try {
            store().deleteEntry(alias);
        } catch (Exception ignored) {
            // Nothing to delete.
        }
    }
}
