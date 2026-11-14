package rine.api;

/** Minimal logger that writes to stderr with a [rine] prefix. */
public final class Log {
    private Log() {}

    public static void info(String msg) {
        System.out.println("[rine] " + msg);
    }

    public static void warn(String msg) {
        System.err.println("[rine] WARN " + msg);
    }

    public static void error(String msg, Throwable t) {
        System.err.println("[rine] ERROR " + msg);
        if (t != null) t.printStackTrace(System.err);
    }
}
