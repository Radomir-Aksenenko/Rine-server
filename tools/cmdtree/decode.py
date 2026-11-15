"""Decode ClientboundCommandsPacket bodies captured from the vanilla 26.3
server (bot<level>-commands.bin, level = op permission level) into one tree
with the minimum permission level for every node.

Output: crates/rine-command/assets/tree.json, a flat node list:
  {"kind": "root"|"literal"|"argument", "name": str, "exec": bool,
   "children": [idx], "redirect": idx|null, "parser": str|null,
   "props": {...}|null, "suggest": str|null, "level": int, "restricted": bool}
Node 0 is the root. Paths are space-joined node names from the root.

Run: python tools/cmdtree/decode.py
"""
import json, os, struct, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
REG = json.load(open(os.path.join(ROOT, "vanilla-data/generated/reports/registries.json")))
ARG_TYPES = {v["protocol_id"]: k for k, v in REG["minecraft:command_argument_type"]["entries"].items()}


class R:
    def __init__(self, b):
        self.b, self.o = b, 0

    def u8(self):
        v = self.b[self.o]; self.o += 1; return v

    def varint(self):
        v = s = 0
        while True:
            x = self.u8(); v |= (x & 0x7F) << s; s += 7
            if not x & 0x80: break
        if v >= 1 << 31: v -= 1 << 32
        return v

    def str(self):
        n = self.varint(); s = self.b[self.o:self.o + n].decode(); self.o += n; return s

    def fmt(self, f):
        n = struct.calcsize(f); v = struct.unpack(">" + f, self.b[self.o:self.o + n])[0]; self.o += n; return v


def props(r, parser):
    if parser in ("brigadier:float", "brigadier:double", "brigadier:integer", "brigadier:long"):
        f = r.u8(); c = {"brigadier:float": "f", "brigadier:double": "d", "brigadier:integer": "i", "brigadier:long": "q"}[parser]
        p = {}
        if f & 1: p["min"] = r.fmt(c)
        if f & 2: p["max"] = r.fmt(c)
        return p
    if parser == "brigadier:string":
        return {"type": ["word", "phrase", "greedy"][r.varint()]}
    if parser == "minecraft:entity":
        f = r.u8(); return {"single": bool(f & 1), "players": bool(f & 2)}
    if parser == "minecraft:score_holder":
        f = r.u8(); return {"multiple": bool(f & 1)}
    if parser == "minecraft:time":
        return {"min": r.fmt("i")}
    if parser in ("minecraft:resource_or_tag", "minecraft:resource_or_tag_key", "minecraft:resource",
                  "minecraft:resource_key", "minecraft:resource_selector"):
        return {"registry": r.str()}
    return None


def decode(path):
    r = R(open(path, "rb").read())
    n = r.varint(); nodes = []
    for _ in range(n):
        fl = r.u8(); kids = [r.varint() for _ in range(r.varint())]
        red = r.varint() if fl & 8 else None
        kind = fl & 3; nd = {"kind": ["root", "literal", "argument"][kind], "exec": bool(fl & 4),
                             "children": kids, "redirect": red, "restricted": bool(fl & 0x20),
                             "name": "", "parser": None, "props": None, "suggest": None}
        if kind == 1: nd["name"] = r.str()
        elif kind == 2:
            nd["name"] = r.str(); nd["parser"] = ARG_TYPES[r.varint()]; nd["props"] = props(r, nd["parser"])
            if fl & 0x10: nd["suggest"] = r.str()
        nodes.append(nd)
    root = r.varint(); assert r.o == len(r.b), (path, r.o, len(r.b))
    return nodes, root


def paths(nodes, root):
    """path -> node index (redirect targets are reached through their own path)."""
    out = {}; stack = [(root, "")]
    while stack:
        i, p = stack.pop()
        if p in out: continue
        out[p] = i
        for c in nodes[i]["children"]:
            stack.append((c, (p + " " + nodes[c]["name"]).strip()))
    return out


def main():
    trees = {}
    for lvl in range(5):
        f = os.path.join(HERE, f"bot{lvl}-commands.bin")
        trees[lvl] = decode(f)
    full, froot = trees[4]
    assert froot == 0 or True
    fp = paths(full, froot)
    inv = {i: p for p, i in fp.items()}
    levels = {}
    for lvl in range(4, -1, -1):
        nodes, root = trees[lvl]
        for p in paths(nodes, root):
            levels[p] = lvl
    # reorder so the root is index 0, keep vanilla child order
    order = [froot] + [i for i in range(len(full)) if i != froot]
    remap = {old: new for new, old in enumerate(order)}
    out = []
    for old in order:
        nd = dict(full[old])
        nd["children"] = [remap[c] for c in nd["children"]]
        if nd["redirect"] is not None: nd["redirect"] = remap[nd["redirect"]]
        nd["level"] = levels.get(inv.get(old, None), 4)
        nd["path"] = inv.get(old, "")
        out.append(nd)
    dst = os.path.join(ROOT, "crates/rine-command/assets/tree.json")
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    json.dump(out, open(dst, "w"), indent=0)
    lit = [out[c]["name"] + ":" + str(out[c]["level"]) for c in out[0]["children"]]
    print(len(out), "nodes;", " ".join(lit))


if __name__ == "__main__":
    main()
