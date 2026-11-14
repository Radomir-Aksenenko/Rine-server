// Writes the tree golden: a datapack in which every overworld biome decorates only synthetic tree placed
// features (one per tree and fallen tree configured feature the Rust port supports), plus the matching
// feature bundle for the Rust side. Everything else is removed from the biome lists and carvers are emptied,
// so a golden chunk differs from our decoration only in the tree code.
//
//   node tools/noise-golden/make_tree_golden.js <outDir> <bundleOut>
//
// Placement of each synthetic feature: `count` 2, `in_square`, `height_range` uniform in [-10, 100]. Trees
// may float (no ground check): TreeFeature.place does not check the ground, only the trunk base is tested.
// The step is VEGETAL_DECORATION (index 9) in every overworld biome. Unsupported configs (poplar, pale moss,
// shelf mushroom, bending) are skipped and listed on stdout.
'use strict';
const fs = require('fs');
const path = require('path');

const REPO = path.resolve(__dirname, '..', '..');
const VANILLA_GENERATED = process.env.RINE_VANILLA_GENERATED || path.join(REPO, 'vanilla-data', 'generated');
const FEATURES = path.join(VANILLA_GENERATED, 'data', 'minecraft', 'worldgen', 'feature');
const BIOME_DIR = path.join(VANILLA_GENERATED, 'data', 'minecraft', 'worldgen', 'biome');
const BUNDLE = path.join(REPO, 'crates', 'rine-worldgen', 'assets', 'worldgen', 'features.json');

const [out, bundleOut] = process.argv.slice(2);
if (!bundleOut) throw new Error('usage: make_tree_golden.js <outDir> <bundleOut>');

const SUPPORTED_TRUNK = ['straight', 'forking', 'giant', 'mega_jungle', 'dark_oak', 'fancy', 'upwards_branching', 'cherry'];
const SUPPORTED_FOLIAGE = ['blob', 'spruce', 'pine', 'acacia', 'bush', 'fancy', 'jungle', 'mega_pine', 'dark_oak', 'random_spread', 'cherry'];
const SUPPORTED_DECORATOR = ['leave_vine', 'trunk_vine', 'cocoa', 'beehive', 'attached_to_leaves', 'place_on_ground', 'alter_ground', 'creaking_heart', 'attached_to_logs'];

function supported(json) {
    const short = (t) => (t || '').replace('minecraft:', '');
    if (json.type === 'minecraft:fallen_tree') {
        return (json.log_decorators || []).concat(json.stump_decorators || []).every((d) => SUPPORTED_DECORATOR.includes(short(d.type)));
    }
    const trunk = short(json.trunk_placer && json.trunk_placer.type).replace('_trunk_placer', '');
    const foliage = short(json.foliage_placer && json.foliage_placer.type).replace('_foliage_placer', '');
    if (!SUPPORTED_TRUNK.includes(trunk) || !SUPPORTED_FOLIAGE.includes(foliage)) return false;
    return (json.decorators || []).every((d) => SUPPORTED_DECORATOR.includes(short(d.type)));
}

const bundle = JSON.parse(fs.readFileSync(BUNDLE, 'utf8'));
const trees = [];
for (const f of fs.readdirSync(FEATURES).sort()) {
    if (!f.endsWith('.json')) continue;
    const json = JSON.parse(fs.readFileSync(path.join(FEATURES, f), 'utf8'));
    if (json.type !== 'minecraft:tree' && json.type !== 'minecraft:fallen_tree') continue;
    const name = f.replace(/\.json$/, '');
    if (!supported(json)) {
        console.log(`skipped minecraft:${name}`);
        continue;
    }
    trees.push(name);
}

const PLACEMENT = [
    { type: 'minecraft:count', count: 2 },
    { type: 'minecraft:in_square' },
    {
        type: 'minecraft:height_range',
        height: {
            type: 'minecraft:uniform',
            min_inclusive: { absolute: -10 },
            max_inclusive: { absolute: 100 },
        },
    },
];
const STEP = 9;
const placedNames = trees.map((t) => `rine:tree_${t}`);
const placedJson = {};
for (const t of trees) placedJson[`rine:tree_${t}`] = { feature: `minecraft:${t}`, placement: PLACEMENT };

// Datapack: placed features plus every overworld biome with only the synthetic list at STEP.
const packDir = path.join(out, 'data', 'minecraft', 'worldgen', 'biome');
fs.mkdirSync(packDir, { recursive: true });
fs.mkdirSync(path.join(out, 'data', 'rine', 'worldgen', 'placed_feature'), { recursive: true });
fs.writeFileSync(
    path.join(out, 'pack.mcmeta'),
    JSON.stringify({ pack: { description: 'rine tree golden', pack_format: 81, supported_formats: [1, 81] } }, null, 2)
);
for (const t of trees) {
    fs.writeFileSync(
        path.join(out, 'data', 'rine', 'worldgen', 'placed_feature', `tree_${t}.json`),
        JSON.stringify(placedJson[`rine:tree_${t}`])
    );
}
const biomeNames = Object.keys(bundle.biomes);
const steps = () => Array.from({ length: 11 }, (_, i) => (i === STEP ? placedNames.slice() : []));
for (const name of biomeNames) {
    const file = path.join(BIOME_DIR, name.replace('minecraft:', '') + '.json');
    const biome = JSON.parse(fs.readFileSync(file, 'utf8'));
    biome.features = steps();
    if (Array.isArray(biome.carvers)) {
        biome.carvers = [];
    } else if (biome.carvers && typeof biome.carvers === 'object') {
        for (const k of Object.keys(biome.carvers)) biome.carvers[k] = [];
    }
    fs.writeFileSync(path.join(packDir, name.replace('minecraft:', '') + '.json'), JSON.stringify(biome));
}

// Rust side: the same placed features and biome lists, with the full configured/tag/state bundle.
const treeBundle = Object.assign({}, bundle, {
    biomes: Object.fromEntries(biomeNames.map((n) => [n, steps()])),
    placed: placedJson,
});
fs.writeFileSync(bundleOut, JSON.stringify(treeBundle));
console.log(`tree golden: ${trees.length} tree configs, datapack ${out}, bundle ${bundleOut}`);
