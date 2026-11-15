"""Golden comparison of command output: vanilla 26.3 vs Rine.

Usage:
  python tools/cmdtree/golden.py run <commands.txt> [--rine-port 25570] [--vanilla-port 25566]
  python tools/cmdtree/golden.py show <bot0-chat.bin>

`run` sends the same commands (one per line) from bot0 to both servers via
rine-testbot (RINE_DUMP_DIR / RINE_CMD_FILE), decodes every system chat and
command suggestion packet, groups them by command and prints a diff.
The Rine server must already run on --rine-port (never 25565).
UUIDs and timestamps are normalised.
"""
import json, os, re, struct, subprocess, sys, difflib

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
LANG = json.load(open(os.path.join(ROOT, "crates/rine-command/assets/en_us.json"), encoding="utf-8"))
BOT = os.path.join(ROOT, "target/release/rine-testbot.exe")


class R:
    def __init__(self, b):
        self.b, self.o = b, 0

    def take(self, n):
        v = self.b[self.o:self.o + n]; self.o += n; return v

    def u8(self): return self.take(1)[0]
    def i8(self): return struct.unpack(">b", self.take(1))[0]
    def i16(self): return struct.unpack(">h", self.take(2))[0]
    def u16(self): return struct.unpack(">H", self.take(2))[0]
    def i32(self): return struct.unpack(">i", self.take(4))[0]
    def i64(self): return struct.unpack(">q", self.take(8))[0]
    def f32(self): return struct.unpack(">f", self.take(4))[0]
    def f64(self): return struct.unpack(">d", self.take(8))[0]

    def varint(self):
        v = s = 0
        while True:
            x = self.u8(); v |= (x & 0x7F) << s; s += 7
            if not x & 0x80: break
        return v - (1 << 32) if v >= 1 << 31 else v

    def mutf8(self):
        n = self.u16(); return self.take(n).decode("utf-8", "replace")

    def str(self):
        n = self.varint(); return self.take(n).decode("utf-8", "replace")


def nbt_payload(r, t):
    if t == 1: return ("b", r.i8())
    if t == 2: return ("s", r.i16())
    if t == 3: return r.i32()
    if t == 4: return ("l", r.i64())
    if t == 5: return ("f", round(r.f32(), 6))
    if t == 6: return ("d", r.f64())
    if t == 7: return ("B", [r.i8() for _ in range(r.i32())])
    if t == 8: return r.mutf8()
    if t == 9:
        et = r.u8(); n = r.i32(); return [nbt_payload(r, et) for _ in range(n)]
    if t == 10:
        d = {}
        while True:
            et = r.u8()
            if et == 0: return d
            k = r.mutf8(); d[k] = nbt_payload(r, et)
    if t == 11: return ("I", [r.i32() for _ in range(r.i32())])
    if t == 12: return ("L", [r.i64() for _ in range(r.i32())])
    raise ValueError(t)


def nbt_network(r):
    t = r.u8()
    return None if t == 0 else nbt_payload(r, t)


UUIDRE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")


def norm(v):
    if isinstance(v, dict):
        out = {}
        for k, x in v.items():
            if k == "uuid":
                out[k] = "<uuid>"
            else:
                out[k] = norm(x)
        return out
    if isinstance(v, list): return [norm(x) for x in v]
    if isinstance(v, tuple):
        if v[0] == "I" and len(v[1]) == 4: return "<uuid>"
        if v[0] == "b": return {"byte": v[1]}
        return list(v)
    if isinstance(v, str):
        v = UUIDRE.sub("<uuid>", v)
        return re.sub(r"\[\d\d:\d\d:\d\d\]", "[time]", v)
    return v


def comp_plain(c):
    """English rendering of a decoded component (for readability)."""
    if isinstance(c, str): return c
    if isinstance(c, list): return "".join(comp_plain(x) for x in c)
    if isinstance(c, tuple): return str(c[1])
    if isinstance(c, (int, float)): return str(c)
    if not isinstance(c, dict): return ""
    s = ""
    if "text" in c: s = c["text"] if isinstance(c["text"], str) else comp_plain(c["text"])
    elif "translate" in c:
        fmt = LANG.get(c["translate"], c.get("fallback", c["translate"]))
        args = [comp_plain(a) for a in c.get("with", [])]
        i = [0]

        def rep(m):
            if m.group(0) == "%%": return "%"
            if m.group(1):
                k = int(m.group(1)) - 1
            else:
                k = i[0]; i[0] += 1
            return args[k] if k < len(args) else ""
        s = re.sub(r"%(?:(\d+)\$)?s|%%", rep, fmt)
    elif "keybind" in c: s = c["keybind"]
    for e in c.get("extra", []):
        s += comp_plain(e)
    return s


def decode_chat(path):
    """[(command, [messages])]; messages before the first command go under ''."""
    out = [["", []]]
    b = open(path, "rb").read()
    o = 0
    while o + 8 <= len(b):
        pid, n = struct.unpack("<II", b[o:o + 8]); o += 8
        body = b[o:o + n]; o += n
        if pid == 0xFFFFFFFF:
            out.append([body.decode("utf-8", "replace"), []])
            continue
        r = R(body)
        if pid == 124:  # system_chat
            c = nbt_network(r); overlay = r.u8() != 0
            out[-1][1].append(("overlay " if overlay else "") + json.dumps(norm(c), ensure_ascii=False, sort_keys=True)
                              + "\n      = " + UUIDRE.sub("<uuid>", comp_plain(c)))
        elif pid == 15:  # command_suggestions
            tid = r.varint(); start = r.varint(); ln = r.varint(); cnt = r.varint()
            items = []
            for _ in range(cnt):
                t = r.str(); has = r.u8()
                if has: nbt_network(r)
                items.append(t)
            out[-1][1].append(f"suggestions {start}+{ln}: {items}")
    return out


def report(path):
    lines = []
    for cmd, msgs in decode_chat(path):
        if not cmd: continue
        lines.append(f"> /{cmd}")
        lines.extend("    " + m for m in msgs)
    return lines


def run_bot(port, cmds, dump):
    os.makedirs(dump, exist_ok=True)
    for f in os.listdir(dump):
        os.remove(os.path.join(dump, f))
    env = dict(os.environ, RINE_DUMP_DIR=dump, RINE_CMD_FILE=cmds, RINE_CMD_GAP_MS="400")
    n = sum(1 for l in open(cmds, encoding="utf-8") if l.strip() and not l.startswith("#"))
    secs = 4 + n * 0.4 + 2
    subprocess.run([BOT, f"127.0.0.1:{port}", "--bots", "2", "--seconds", str(secs), "--cmd-delay", "2500"],
                   env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    return os.path.join(dump, "bot0-chat.bin")


def main():
    if sys.argv[1] == "show":
        print("\n".join(report(sys.argv[2])))
        return
    cmds = os.path.abspath(sys.argv[2])
    rp = 25570; vp = 25566
    if "--rine-port" in sys.argv: rp = int(sys.argv[sys.argv.index("--rine-port") + 1])
    if "--vanilla-port" in sys.argv: vp = int(sys.argv[sys.argv.index("--vanilla-port") + 1])
    base = os.path.join(HERE, "golden-out")
    v = report(run_bot(vp, cmds, os.path.join(base, "vanilla")))
    r = report(run_bot(rp, cmds, os.path.join(base, "rine")))
    open(os.path.join(base, "vanilla.txt"), "w", encoding="utf-8").write("\n".join(v))
    open(os.path.join(base, "rine.txt"), "w", encoding="utf-8").write("\n".join(r))
    d = list(difflib.unified_diff(v, r, "vanilla", "rine", lineterm="", n=2))
    cmdsv = sum(1 for l in v if l.startswith("> /"))
    print(f"commands: {cmdsv}, diff lines: {len(d)}")
    print("\n".join(d))


if __name__ == "__main__":
    main()
