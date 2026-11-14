// Bundles the vanilla decoration data the Rust feature port needs into one JSON file:
// crates/rine-worldgen/assets/worldgen/features.json.
//
// Contents (only what the overworld biome list reaches):
//   biomes:     biome name -> per-step lists of placed feature names (biome JSON "features", order kept)
//   placed:     placed feature name -> { feature, placement }   (feature: name string or inline object)
//   configured: configured feature name -> { type, config }
//   inline:     inline placed features nested in configured feature configs, keyed by JSON text
//
// Biome names come from the bundled overworld_biomes.txt (`B` lines), so the list matches the
// biome source of the Rust port. Usage: node tools/noise-golden/bundle_features.js
'use strict';
const fs = require('fs');
const path = require('path');

const REPO = path.resolve(__dirname, '..', '..');
// vanilla-data is git-ignored and lives in the main checkout; RINE_VANILLA_GENERATED overrides the path.
const VANILLA_GENERATED = process.env.RINE_VANILLA_GENERATED || path.join(REPO, 'vanilla-data', 'generated');
const DATA = path.join(VANILLA_GENERATED, 'data', 'minecraft', 'worldgen');
const BIOMES_TXT = path.join(REPO, 'crates', 'rine-worldgen', 'assets', 'worldgen', 'overworld_biomes.txt');
const OUT = path.join(REPO, 'crates', 'rine-worldgen', 'assets', 'worldgen', 'features.json');

const read = (p) => JSON.parse(fs.readFileSync(p, 'utf8'));
const biomeNames = fs
    .readFileSync(BIOMES_TXT, 'utf8')
    .split('\n')
    .filter((l) => l.startsWith('B '))
    .map((l) => l.split(' ')[1]);

const biomes = {};
const placed = {};
const configured = {};
const inline = {};

function assertName(v, where) {
    if (typeof v !== 'string' || !v.startsWith('minecraft:')) {
        throw new Error(`unexpected feature reference at ${where}: ${JSON.stringify(v)}`);
    }
}

function visitPlaced(ref) {
    if (typeof ref === 'string') {
        if (placed[ref]) return;
        const json = read(path.join(DATA, 'placed_feature', ref.replace('minecraft:', '') + '.json'));
        placed[ref] = json;
        visitConfigured(json.feature);
        return;
    }
    if (ref === undefined || ref === null) return;
    // Inline placed feature (object): keyed by its JSON text.
    const key = JSON.stringify(ref);
    if (inline[key]) return;
    inline[key] = ref;
    visitConfigured(ref.feature);
}

function visitConfigured(ref) {
    if (typeof ref === 'string') {
        if (configured[ref]) return;
        const json = read(path.join(DATA, 'feature', ref.replace('minecraft:', '') + '.json'));
        configured[ref] = json;
        visitChildren(json);
        return;
    }
    // Inline configured feature object.
    const key = JSON.stringify(ref);
    if (inline['cfg:' + key]) return;
    inline['cfg:' + key] = ref;
    visitChildren(ref);
}

function visitChildren(cf) {
    const c = cf.config || cf;
    switch (cf.type) {
        case 'minecraft:random_selector':
            (c.features || []).forEach((e) => visitPlaced(e.feature));
            visitPlaced(c.default);
            break;
        case 'minecraft:simple_random_selector':
            (c.features || []).forEach((e) => visitPlaced(e));
            break;
        case 'minecraft:weighted_random_selector':
            (c.features || []).forEach((e) => visitPlaced(e.feature));
            visitPlaced(c.default);
            break;
        case 'minecraft:random_boolean_selector':
            visitPlaced(c.feature_true);
            visitPlaced(c.feature_false);
            break;
        case 'minecraft:vegetation_patch':
        case 'minecraft:waterlogged_vegetation_patch':
            visitPlaced(c.vegetation_feature);
            break;
        case 'minecraft:sequence':
            (c.features || []).forEach(visitPlaced);
            break;
        default:
            break;
    }
}

for (const name of biomeNames) {
    const biome = read(path.join(DATA, 'biome', name.replace('minecraft:', '') + '.json'));
    const steps = biome.features || [];
    const out = steps.map((step, i) =>
        step.map((ref) => {
            assertName(ref, `${name} step ${i}`);
            visitPlaced(ref);
            return ref;
        })
    );
    biomes[name] = out;
}

// Block tags referenced by the bundled predicates (`"tag": "minecraft:x"` and `"#minecraft:x"`),
// flattened to block ids: tags/block/*.json with nested `#` references resolved.
const TAGS = path.join(VANILLA_GENERATED, 'data', 'minecraft', 'tags', 'block');
const blockTags = {};
function flattenTag(name) {
    const key = name.replace(/^#/, '');
    if (blockTags[key]) return blockTags[key];
    const file = path.join(TAGS, key.replace('minecraft:', '') + '.json');
    const json = read(file);
    const out = new Set();
    for (const v of json.values) {
        const entry = typeof v === 'string' ? v : v.id;
        if (entry.startsWith('#')) flattenTag(entry).forEach((b) => out.add(b));
        else out.add(entry);
    }
    blockTags[key] = [...out].sort();
    return blockTags[key];
}
const tagRefs = new Set(['minecraft:blocks_motion_in_heightmap']);
const collectTags = (node) => {
    if (Array.isArray(node)) node.forEach(collectTags);
    else if (node && typeof node === 'object') {
        for (const [k, v] of Object.entries(node)) {
            if ((k === 'tag' || k === 'matching_block_tag') && typeof v === 'string') tagRefs.add(v);
            collectTags(v);
        }
    }
};
collectTags(placed);
collectTags(configured);
for (const t of tagRefs) flattenTag(t);

const bundle = { biomes, placed, configured, inline, blockTags };
fs.mkdirSync(path.dirname(OUT), { recursive: true });
fs.writeFileSync(OUT, JSON.stringify(bundle) + '\n');
console.log(
    `bundled ${biomeNames.length} biomes, ${Object.keys(placed).length} placed, ${Object.keys(configured).length} configured -> ${OUT}`
);
