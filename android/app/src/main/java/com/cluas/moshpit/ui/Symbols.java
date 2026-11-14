package com.cluas.moshpit.ui;

import java.util.HashMap;
import java.util.Map;

/**
 * The SF Symbols the shared screens name ({@code native("moshpit/symbol")}), drawn on Android as text glyphs every
 * system font has. A stand-in until the app bundles an icon font: the shapes are close, not the same.
 * <p>
 * Plain Java (no Android API), so the unit tests cover the table.
 */
public final class Symbols {
    private Symbols() {}

    private static final Map<String, String> GLYPHS = new HashMap<>();

    static {
        String[][] table = {
            {"chevron.right", "›"}, {"chevron.forward", "›"}, {"chevron.backward", "‹"}, {"chevron.left", "‹"},
            {"chevron.down", "⌄"}, {"chevron.up", "⌃"}, {"chevron.up.chevron.down", "⇕"},
            {"plus", "+"}, {"plus.circle", "⊕"}, {"plus.circle.fill", "⊕"}, {"minus.circle.fill", "⊖"},
            {"checkmark", "✓"}, {"checkmark.circle.fill", "✓"}, {"checkmark.seal.fill", "✓"},
            {"checkmark.shield.fill", "✓"}, {"xmark", "✕"}, {"xmark.circle", "⊗"},
            {"trash", "⌫"}, {"trash.fill", "⌫"}, {"doc.on.doc", "⧉"}, {"doc.text", "≣"},
            {"square.and.arrow.up", "⇪"}, {"square.and.arrow.down", "⇩"}, {"square.and.pencil", "✎"},
            {"arrow.clockwise", "↻"}, {"arrow.up.right", "↗"}, {"arrow.up.and.down", "⇅"},
            {"arrow.up.and.down.and.arrow.left.and.right", "✥"}, {"arrow.triangle.branch", "⑂"},
            {"arrow.down.circle", "⇩"}, {"ellipsis", "⋯"}, {"ellipsis.circle", "⋯"},
            {"exclamationmark.triangle.fill", "⚠"}, {"exclamationmark.octagon.fill", "⛔"},
            {"info.circle.fill", "ⓘ"}, {"questionmark", "?"}, {"hand.raised.fill", "✋"},
            {"key", "⚿"}, {"key.fill", "⚿"}, {"key.slash.fill", "⚿"}, {"bolt.fill", "⚡"},
            {"bolt.slash", "⚡"}, {"bolt.slash.fill", "⚡"}, {"bell.slash.fill", "🔕"}, {"moon.zzz", "☾"},
            {"terminal", ">_"}, {"rectangle.split.2x1", "◫"}, {"rectangle.split.2x2", "⊞"},
            {"sidebar.left", "◧"}, {"slider.horizontal.3", "☰"}, {"chart.bar", "▥"},
            {"antenna.radiowaves.left.and.right.slash", "⌁"}, {"square.stack.3d.up.slash", "⧉"},
            {"studio.local", "◎"}, {"sparkles", "✦"}, {"sparkle", "✦"}, {"mic", "🎙"}, {"photo", "▣"},
            {"folder", "▤"}, {"network", "⌘"}, {"power", "▶"}, {"hammer", "⚒"}, {"shippingbox", "▦"},
            {"mediastick", "▮"}, {"number", "#"}, {"apple", ""}, {"gearshape", "⚙"},
            // The terminal screen's.
            {"keyboard", "⌨"}, {"keyboard.chevron.compact.down", "⌄"}, {"arrow.up", "↑"}, {"arrow.down", "↓"},
            {"arrow.left", "←"}, {"arrow.right", "→"}, {"doc.on.clipboard", "⎘"}, {"text.insert", "⎀"},
            {"wifi.exclamationmark", "⚠"}, {"pause.circle.fill", "⏸"}, {"sidebar.leading", "◧"},
            {"square.stack.3d.up", "❐"}, {"squareshape.split.2x2", "⊞"}, {"macwindow", "▭"},
            {"rectangle.grid.2x2", "⊞"}, {"square.on.square", "❐"}, {"chevron.left", "‹"},
        };
        for (String[] row : table) GLYPHS.put(row[0], row[1]);
    }

    /** The glyph standing in for symbol {@code name}; a dot for one the table does not know. */
    public static String glyph(String name) {
        if (name == null || name.isEmpty()) return "";
        String glyph = GLYPHS.get(name);
        if (glyph != null) return glyph;
        // A ".fill" or ".circle" variant of a known symbol is drawn as that symbol.
        int dot = name.lastIndexOf('.');
        return dot > 0 ? glyph(name.substring(0, dot)) : "•";
    }
}
