package com.cluas.moshpit.ui;

/** The screens' colors as Android's: {@code #RRGGBB} or {@code #RRGGBBAA} (alpha last) to ARGB. Plain Java. */
public final class Colors {
    private Colors() {}

    /** The ARGB of {@code value}, or null when it is not one of the two forms. */
    public static Integer parse(Object value) {
        if (!(value instanceof String)) return null;
        String hex = (String) value;
        if (!hex.startsWith("#") || (hex.length() != 7 && hex.length() != 9)) return null;
        for (int i = 1; i < hex.length(); i++) {
            if (Character.digit(hex.charAt(i), 16) < 0) return null;
        }
        long packed = Long.parseLong(hex.substring(1), 16);
        if (hex.length() == 7) return (int) (0xFF000000L | packed);
        return (int) (((packed & 0xFF) << 24) | (packed >> 8));
    }
}
