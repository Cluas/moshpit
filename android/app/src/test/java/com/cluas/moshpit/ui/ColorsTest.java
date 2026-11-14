package com.cluas.moshpit.ui;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import org.junit.Test;

public class ColorsTest {
    @Test
    public void rgbIsOpaque() {
        assertEquals(Integer.valueOf(0xFF53DCC9), Colors.parse("#53DCC9"));
    }

    @Test
    public void theScreensPutAlphaLastAndroidFirst() {
        assertEquals(Integer.valueOf(0x8053DCC9), Colors.parse("#53DCC980"));
        assertEquals(Integer.valueOf(0x00000000), Colors.parse("#00000000"));
    }

    @Test
    public void anythingElseIsNoColor() {
        assertNull(Colors.parse("53DCC9"));
        assertNull(Colors.parse("#53DCC"));
        assertNull(Colors.parse("#53DCCG"));
        assertNull(Colors.parse("#+3DCC9"));
        assertNull(Colors.parse(42));
        assertNull(Colors.parse(null));
    }
}
