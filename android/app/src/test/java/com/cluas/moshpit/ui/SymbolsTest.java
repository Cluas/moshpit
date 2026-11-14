package com.cluas.moshpit.ui;

import static org.junit.Assert.assertEquals;

import org.junit.Test;

public class SymbolsTest {
    @Test
    public void theScreensSymbolsHaveGlyphs() {
        assertEquals("›", Symbols.glyph("chevron.right"));
        assertEquals("+", Symbols.glyph("plus"));
        assertEquals("✓", Symbols.glyph("checkmark"));
        assertEquals(">_", Symbols.glyph("terminal"));
    }

    @Test
    public void aVariantOfAKnownSymbolIsDrawnAsIt() {
        assertEquals(Symbols.glyph("trash"), Symbols.glyph("trash.fill"));
        assertEquals(Symbols.glyph("folder"), Symbols.glyph("folder.badge.plus"));
    }

    @Test
    public void anUnknownSymbolIsADotAndNoneIsNothing() {
        assertEquals("•", Symbols.glyph("tortoise"));
        assertEquals("•", Symbols.glyph("x.y.z"));
        assertEquals("", Symbols.glyph(""));
        assertEquals("", Symbols.glyph(null));
    }
}
