package rine.api;

/**
 * Text encoding shared with the Rust side. Fields are separated by U+0001.
 * Callers never need this class; it exists so both halves agree on the format.
 */
public final class Wire {
    public static final String SEP = "\u0001";

    private Wire() {}

    /** Packs fields into one string; separators inside fields are replaced with spaces. */
    public static String pack(String... fields) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < fields.length; i++) {
            if (i > 0) sb.append(SEP);
            sb.append(clean(fields[i]));
        }
        return sb.toString();
    }

    public static String clean(String s) {
        return s == null ? "" : s.replace(SEP, " ");
    }

    public static String[] split(String packed) {
        return packed.split(SEP, -1);
    }
}
