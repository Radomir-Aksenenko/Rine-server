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
        case 'minecraft:root_system':
            visitPlaced(c.feature);
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

// Every tree and fallen tree configured feature, reached or not: the tree golden places them directly
// through synthetic placed features (tools/noise-golden/make_tree_golden.js), so their providers, tags and
// block states must be bundled too.
for (const f of fs.readdirSync(path.join(DATA, 'feature'))) {
    if (!f.endsWith('.json')) continue;
    const json = read(path.join(DATA, 'feature', f));
    if (json.type !== 'minecraft:tree' && json.type !== 'minecraft:fallen_tree') continue;
    const ref = 'minecraft:' + f.replace(/\.json$/, '');
    if (!configured[ref]) configured[ref] = json;
}

// Configured features that the block code places by name (`blocks/growth.rs`, bone meal and growth of the
// huge mushrooms). Only the configured entries: no biome list reaches them.
const BLOCK_FEATURES = ['minecraft:huge_red_mushroom', 'minecraft:huge_brown_mushroom'];
for (const ref of BLOCK_FEATURES) {
    if (!configured[ref]) configured[ref] = read(path.join(DATA, 'feature', ref.replace('minecraft:', '') + '.json'));
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
const tagRefs = new Set(['minecraft:blocks_motion_in_heightmap', 'minecraft:blocks_motion_in_heightmap_no_leaves']);
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


// Block state providers referenced by name (`"minecraft:x"` in a `*provider` key) and by the tree
// decorators (`worldgen/block_state_provider/<name>.json`), with their own tag references.
const PROVIDER_DIR = path.join(DATA, 'block_state_provider');
const blockStateProviders = {};
function visitProviderRef(ref) {
    if (typeof ref !== 'string' || !ref.startsWith('minecraft:') || blockStateProviders[ref]) return;
    const file = path.join(PROVIDER_DIR, ref.replace('minecraft:', '') + '.json');
    if (!fs.existsSync(file)) return;
    const json = read(file);
    blockStateProviders[ref] = json;
    collectProviderRefs(json);
}
function collectProviderRefs(node) {
    if (Array.isArray(node)) node.forEach(collectProviderRefs);
    else if (node && typeof node === 'object') {
        for (const [k, v] of Object.entries(node)) {
            if (k === 'id' || k === 'type' || k === 'predicate_type' || k === 'tag') continue;
            if (typeof v === 'string' && v.startsWith('minecraft:')) visitProviderRef(v);
            else collectProviderRefs(v);
        }
    }
}
const providerKeyRefs = (node) => {
    if (Array.isArray(node)) node.forEach(providerKeyRefs);
    else if (node && typeof node === 'object') {
        for (const [k, v] of Object.entries(node)) {
            if (/provider$/.test(k) && typeof v === 'string') visitProviderRef(v);
            else providerKeyRefs(v);
        }
    }
};
providerKeyRefs(configured);
providerKeyRefs(placed);
collectProviderRefs(blockStateProviders);
collectTags(blockStateProviders);

// Tags that the tree and fallen tree code tests from Java (BlockTags.LEAVES, LOGS, ...).
['minecraft:leaves', 'minecraft:logs', 'minecraft:replaceable_by_trees', 'minecraft:prevents_nearby_leaf_decay',
 'minecraft:mangrove_roots_can_grow_through', 'minecraft:mangrove_logs_can_grow_through',
 'minecraft:cannot_replace_below_tree_trunk', 'minecraft:beneath_tree_podzol_replaceable',
 'minecraft:blocks_motion_no_leaves', 'minecraft:snow', 'minecraft:supports_vegetation', 'minecraft:supports_mangrove_propagule', 'minecraft:supports_hanging_mangrove_propagule',
 // Vegetation survival (crates/rine-worldgen/src/feature/vegetation.rs): dry vegetation and seagrass support.
 'minecraft:supports_dry_vegetation', 'minecraft:cannot_support_seagrass',
 'minecraft:replaceable_by_mushrooms'].forEach((t) => tagRefs.add(t));
for (const t of tagRefs) flattenTag(t);

// Block property tables from the 26.3 block report: ordered property values and the default state, for
// every block named in the bundled data plus the blocks the tree decorators write.
const REPORT = path.join(VANILLA_GENERATED, 'reports', 'blocks.json');
const report = read(REPORT);
const blockNames = new Set();
const collectNames = (node) => {
    if (typeof node === 'string') {
        const m = node.match(/^(?:#)?(minecraft:[a-z0-9_]+)/);
        if (m) blockNames.add(m[1]);
    } else if (Array.isArray(node)) node.forEach(collectNames);
    else if (node && typeof node === 'object') Object.values(node).forEach(collectNames);
};
collectNames(configured);
collectNames(blockStateProviders);
collectNames(placed);
[
    'minecraft:vine', 'minecraft:cocoa', 'minecraft:bee_nest', 'minecraft:creaking_heart',
    'minecraft:pale_hanging_moss', 'minecraft:mangrove_roots', 'minecraft:muddy_mangrove_roots',
    'minecraft:mangrove_propagule', 'minecraft:moss_carpet', 'minecraft:podzol', 'minecraft:dirt',
    'minecraft:pale_moss_carpet', 'minecraft:leaf_litter',
].forEach((n) => blockNames.add(n));
const blockProps = {};
for (const name of [...blockNames].sort()) {
    const entry = report[name];
    if (!entry) continue;
    const props = entry.properties || {};
    const states = entry.states || [];
    const def = states.find((s) => s.default) || states[0];
    blockProps[name] = {
        properties: props,
        default: def ? def.properties || {} : {},
    };
}

const bundle = { biomes, placed, configured, inline, blockTags, blockStateProviders, blockProps };
fs.mkdirSync(path.dirname(OUT), { recursive: true });
fs.writeFileSync(OUT, JSON.stringify(bundle) + '\n');
console.log(
    `bundled ${biomeNames.length} biomes, ${Object.keys(placed).length} placed, ${Object.keys(configured).length} configured -> ${OUT}`
);
