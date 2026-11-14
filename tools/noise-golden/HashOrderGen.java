import java.io.*;
import java.util.*;

/** Drives a real java.util.HashSet with BlockPos-like keys and records iteration hashes after each op. */
public class HashOrderGen {
    static final class P {
        final int x, y, z;
        P(int x, int y, int z) { this.x = x; this.y = y; this.z = z; }
        @Override public int hashCode() { return (this.y + this.z * 31) * 31 + this.x; }
        @Override public boolean equals(Object o) {
            if (!(o instanceof P)) return false;
            P p = (P) o;
            return x == p.x && y == p.y && z == p.z;
        }
    }

    static java.lang.reflect.Field find(Class<?> c, String name) throws NoSuchFieldException {
        for (Class<?> k = c; k != null; k = k.getSuperclass()) {
            try {
                java.lang.reflect.Field f = k.getDeclaredField(name);
                f.setAccessible(true);
                return f;
            } catch (NoSuchFieldException e) {
                // continue with the superclass
            }
        }
        throw new NoSuchFieldException(name);
    }

    static Object g(Object o, String n) throws Exception {
        return find(o.getClass(), n).get(o);
    }

    static void dumpNode(Object n, StringBuilder sb) throws Exception {
        if (n == null) {
            sb.append('-');
            return;
        }
        P k = (P) g(n, "key");
        sb.append(k.x).append(',').append(k.y).append(',').append(k.z);
        sb.append(((Boolean) g(n, "red")) ? 'R' : 'B');
        sb.append('(');
        dumpNode(g(n, "left"), sb);
        sb.append(',');
        dumpNode(g(n, "right"), sb);
        sb.append(')');
    }

    /** Tree shape of every tree bin: head key, then the red-black tree from its root. */
    static String shape(HashSet<P> set) throws Exception {
        Object map = g(set, "map");
        Object[] tab = (Object[]) g(map, "table");
        StringBuilder sb = new StringBuilder();
        if (tab == null) return "";
        for (int i = 0; i < tab.length; i++) {
            Object head = tab[i];
            if (head == null || !head.getClass().getSimpleName().equals("TreeNode")) continue;
            Object root = head;
            while (g(root, "parent") != null) root = g(root, "parent");
            P hk = (P) g(head, "key");
            sb.append(i).append('[').append(hk.x).append(',').append(hk.y).append(',').append(hk.z).append("]:");
            dumpNode(root, sb);
            sb.append(" | ");
        }
        return sb.toString();
    }

    static long fnv(Collection<P> c) {
        long h = 1469598103934665603L;
        for (P p : c) {
            for (long v : new long[] { p.x, p.y, p.z }) {
                h ^= v;
                h *= 1099511628211L;
            }
            h ^= 0x9e3779b97f4a7c15L;
            h *= 1099511628211L;
        }
        h ^= c.size();
        h *= 1099511628211L;
        return h;
    }

    public static void main(String[] args) throws Exception {
        long seed = Long.parseLong(args[0]);
        int steps = Integer.parseInt(args[1]);
        Random rng = new Random(seed);
        HashSet<P> set = new HashSet<>();
        Map<Integer, P> byHash = new HashMap<>();
        StringBuilder out = new StringBuilder();
        for (int step = 0; step < steps; step++) {
            int r = rng.nextInt(100);
            if (r < 62) {
                int x, y, z;
                if (rng.nextBoolean()) {
                    // Same low six hash bits (x, z multiples of 64): every such key falls into one of two
                    // buckets at table size 64, so bins reach 9 nodes and Java treeifies them.
                    x = 64 * (rng.nextInt(128) - 64);
                    y = 0;
                    z = 64 * (rng.nextInt(5) - 2);
                } else {
                    x = rng.nextInt(256) - 128;
                    y = rng.nextInt(3);
                    z = rng.nextInt(256) - 128;
                }
                P p = new P(x, y, z);
                if (byHash.containsKey(p.hashCode())) continue; // no identity-hash ties
                set.add(p);
                byHash.put(p.hashCode(), p);
                out.append("A ").append(x).append(' ').append(y).append(' ').append(z).append('\n');
            } else if (r < 88) {
                if (set.isEmpty()) continue;
                int k = rng.nextInt(set.size());
                P victim = null;
                int i = 0;
                for (P p : set) { if (i++ == k) { victim = p; break; } }
                set.remove(victim);
                byHash.remove(victim.hashCode());
                out.append("R ").append(victim.x).append(' ').append(victim.y).append(' ').append(victim.z).append('\n');
            } else {
                if (set.isEmpty()) continue;
                Iterator<P> it = set.iterator();
                P q = it.next();
                it.remove();
                byHash.remove(q.hashCode());
                out.append("P ").append(q.x).append(' ').append(q.y).append(' ').append(q.z).append('\n');
            }
            out.append("H ").append(Long.toHexString(fnv(set))).append(' ').append(set.size()).append('\n');
            if (args.length > 3) {
                out.append("S ").append(shape(set)).append('\n');
                out.append("O");
                for (P p : set) out.append(' ').append(p.x).append(',').append(p.y).append(',').append(p.z);
                out.append('\n');
            }
        }
        try (Writer w = new OutputStreamWriter(new FileOutputStream(args[2]), "UTF-8")) {
            w.write(out.toString());
        }
    }
}
