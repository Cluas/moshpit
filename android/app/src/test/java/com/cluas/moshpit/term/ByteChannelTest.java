package com.cluas.moshpit.term;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

/** What a read of the byte channel says (crates/android/src/android/stream.rs encodes it). */
public class ByteChannelTest {
    @Test
    public void aPlainReadIsItsByteCount() {
        assertEquals(0, ByteChannel.bytesOf(0));
        assertEquals(65536, ByteChannel.bytesOf(65536));
        assertFalse(ByteChannel.marked(0));
        assertFalse(ByteChannel.marked(17));
    }

    @Test
    public void aReadThatStoppedAtAMarkSaysSoWithItsBytes() {
        // -(n + 1): n bytes, then a mark.
        assertTrue(ByteChannel.marked(-1));
        assertEquals(0, ByteChannel.bytesOf(-1));
        assertTrue(ByteChannel.marked(-18));
        assertEquals(17, ByteChannel.bytesOf(-18));
    }
}
