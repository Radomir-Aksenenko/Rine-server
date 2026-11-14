"""Generate compact data assets for rine-data from the vanilla 26.3 jar.

Inputs:  vanilla-data/generated/reports/{blocks,registries}.json
         vanilla-data/versions/26.3/server-26.3.jar (data/minecraft/**)
         vanilla-data/dump/*.json (from tools/datadump, optional here)
Outputs: crates/rine-data/assets/*.json
"""
import json, os, sys, zipfile
from collections import defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REP = os.path.join(ROOT, "vanilla-data", "generated", "reports")
JAR = os.path.join(ROOT, "vanilla-data", "versions", "26.3", "server-26.3.jar")
OUT = os.path.join(ROOT, "crates", "rine-data", "assets")
os.makedirs(OUT, exist_ok=True)

def dump(name, obj):
    with open(os.path.join(OUT, name), "w", encoding="utf-8") as f:
        json.dump(obj, f, separators=(",", ":"), ensure_ascii=False)
    print("wrote", name, os.path.getsize(os.path.join(OUT, name)))

# ---- built-in registries: name lists in protocol id order
regs = json.load(open(os.path.join(REP, "registries.json"), encoding="utf-8"))
out_regs = {}
for rname, r in regs.items():
    ents = sorted(r["entries"].items(), key=lambda kv: kv[1]["protocol_id"])
    names = [k for k, _ in ents]
    assert all(v["protocol_id"] == i for i, (_, v) in enumerate(ents)), rname
    out_regs[rname] = names
dump("registries.json", out_regs)

# ---- blocks: properties + state ranges, verify id formula (last prop varies fastest)
blocks = json.load(open(os.path.join(REP, "blocks.json"), encoding="utf-8"))
out_blocks = []
block_order = out_regs["minecraft:block"]
for bname in block_order:
    b = blocks[bname]
    # vanilla StateDefinition orders properties by name; last varies fastest
    props = sorted([[k, v] for k, v in b.get("properties", {}).items()], key=lambda kv: kv[0])
    states = b["states"]
    ids = [s["id"] for s in states]
    lo = min(ids)
    default = next(s["id"] for s in states if s.get("default"))
    # verify ordering
    strides = []
    acc = 1
    for k, v in reversed(props):
        strides.insert(0, acc)
        acc *= len(v)
    assert acc == len(states), bname
    for s in states:
        idx = 0
        for (k, vals), st in zip(props, strides):
            idx += vals.index(s["properties"][k]) * st
        assert s["id"] == lo + idx, (bname, s)
    out_blocks.append({"n": bname, "p": props, "min": lo, "d": default})
dump("blocks.json", out_blocks)

# ---- jar data
z = zipfile.ZipFile(JAR)
names = z.namelist()

# datapack registry entry lists (sorted by id, like vanilla's resource listing)
dp = defaultdict(list)
for n in names:
    if not n.startswith("data/minecraft/") or not n.endswith(".json"):
        continue
    rest = n[len("data/minecraft/"):]
    if rest.startswith("tags/"):
        continue
    parts = rest.split("/")
    for depth in (1, 2):  # e.g. chat_type/x.json or worldgen/biome/x.json
        if len(parts) > depth:
            reg = "/".join(parts[:depth])
            ent = "/".join(parts[depth:])[:-5]
            dp[reg].append("minecraft:" + ent)
out_dp = {("minecraft:" + k): sorted(v) for k, v in dp.items()}
dump("datapack_registries.json", out_dp)

# tags: resolve recursively into name lists per registry
raw = {}
for n in names:
    if n.startswith("data/minecraft/tags/") and n.endswith(".json"):
        rest = n[len("data/minecraft/tags/"):-5]
        parts = rest.split("/")
        # registry dir may be nested (worldgen/biome); try longest known prefix
        raw[rest] = json.loads(z.read(n))

known_regs = set(r[len("minecraft:"):] for r in list(out_regs) + list(out_dp))
by_reg = defaultdict(dict)
for rest, data in raw.items():
    parts = rest.split("/")
    reg = None
    for d in (2, 1):
        if "/".join(parts[:d]) in known_regs and len(parts) > d:
            reg = "/".join(parts[:d])
            tag = "/".join(parts[d:])
            break
    if reg is None:
        print("skip tag (unknown registry):", rest, file=sys.stderr)
        continue
    by_reg[reg]["minecraft:" + tag] = data

def resolve(reg, tag, seen):
    data = by_reg[reg].get(tag)
    if data is None:
        return []
    res = []
    for v in data.get("values", []):
        req = True
        if isinstance(v, dict):
            req = v.get("required", True)
            v = v["id"]
        if v.startswith("#"):
            t = v[1:]
            if t in seen:
                continue
            res.extend(resolve(reg, t, seen | {t}))
        else:
            res.append(v)
    out = []
    s = set()
    for x in res:
        if x not in s:
            s.add(x)
            out.append(x)
    return out

out_tags = {}
for reg, tags in by_reg.items():
    out_tags["minecraft:" + reg] = {t: resolve(reg, t, {t}) for t in sorted(tags)}
dump("tags.json", out_tags)

# loot tables for blocks (raw json, evaluated in Rust) and recipes (raw)
loot = {}
for n in names:
    if n.startswith("data/minecraft/loot_table/blocks/") and n.endswith(".json"):
        loot["minecraft:" + n[len("data/minecraft/loot_table/blocks/"):-5]] = json.loads(z.read(n))
dump("block_loot.json", loot)
recipes = {}
for n in names:
    if n.startswith("data/minecraft/recipe/") and n.endswith(".json"):
        recipes["minecraft:" + n[len("data/minecraft/recipe/"):-5]] = json.loads(z.read(n))
dump("recipes.json", recipes)

# merge datadump outputs if present
DUMP = os.path.join(ROOT, "vanilla-data", "dump")
for f in ("blocks_ext.json", "items.json", "entities.json"):
    p = os.path.join(DUMP, f)
    if os.path.exists(p):
        data = json.load(open(p, encoding="utf-8"))
        dump(f, data)
    else:
        print("missing dump", f, file=sys.stderr)
