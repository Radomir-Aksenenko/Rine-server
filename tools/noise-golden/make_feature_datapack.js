// Writes a datapack that makes the vanilla server decorate only the placed features the Rust port
// allows: every overworld biome keeps only the allowed placed features in its `features` lists (same
// step structure), and carvers are emptied. Structures are disabled separately in server.properties.
//
// The allow rule is shared with crates/rine-worldgen/tests/feature_golden.rs: a placed feature is allowed
// when its configured feature is `minecraft:ore`. Usage:
//   node tools/noise-golden/make_feature_datapack.js <outDir>
'use strict';
const fs = require('fs');
const path = require('path');

const REPO = path.resolve(__dirname, '..', '..');
const VANILLA_GENERATED = process.env.RINE_VANILLA_GENERATED || path.join(REPO, 'vanilla-data', 'generated');
const DATA = path.join(VANILLA_GENERATED, 'data', 'minecraft', 'worldgen');
const BUNDLE = path.join(REPO, 'crates', 'rine-worldgen', 'assets', 'worldgen', 'features.json');

const out = process.argv[2];
if (!out) throw new Error('usage: make_feature_datapack.js <outDir>');
const bundle = JSON.parse(fs.readFileSync(BUNDLE, 'utf8'));

// Shared allow rule (crates/rine-worldgen/assets/worldgen/ported_features.json): a top-level placed
// feature is allowed when every placement is ported (biome only at top level) and every configured
// feature in its subtree is ported. Nested placed features of selectors and sequences are checked the same way.
const PORTED = JSON.parse(fs.readFileSync(path.join(REPO, "crates", "rine-worldgen", "assets", "worldgen", "ported_features.json"), "utf8"));
const placementOk = (list, top) =>
    (list || []).every((m) => PORTED.placements.includes(m.type) && (top || m.type !== "minecraft:biome"));
const featureOk = (ref) => {
    const cfg = typeof ref === "string" ? bundle.configured[ref] : ref;
    if (!cfg || !PORTED.types.includes(cfg.type)) return false;
    const sub = (r) => subOk(r);
    switch (cfg.type) {
        case "minecraft:random_boolean_selector":
            return sub(cfg.feature_true) && sub(cfg.feature_false);
        case "minecraft:simple_random_selector":
        case "minecraft:sequence":
            return cfg.features.every(sub);
        case "minecraft:weighted_random_selector":
            return cfg.features.every((e) => sub(e.data));
        default:
            return true;
    }
};
const placedOk = (p, top) => !!p && placementOk(p.placement, top) && featureOk(p.feature);
const subOk = (r) => (typeof r === "string" ? placedOk(bundle.placed[r], false) : placedOk(r, false));
const allowed = (placedName) => placedOk(bundle.placed[placedName], true);

const packDir = path.join(out, 'data', 'minecraft', 'worldgen', 'biome');
fs.mkdirSync(packDir, { recursive: true });
fs.writeFileSync(
    path.join(out, 'pack.mcmeta'),
    JSON.stringify({ pack: { description: 'rine feature golden', pack_format: 81, supported_formats: [1, 81] } }, null, 2)
);

for (const name of Object.keys(bundle.biomes)) {
    const file = path.join(DATA, 'biome', name.replace('minecraft:', '') + '.json');
    const biome = JSON.parse(fs.readFileSync(file, 'utf8'));
    biome.features = (biome.features || []).map((step) => step.filter(allowed));
    if (Array.isArray(biome.carvers)) {
        biome.carvers = [];
    } else if (biome.carvers && typeof biome.carvers === 'object') {
        for (const k of Object.keys(biome.carvers)) biome.carvers[k] = [];
    }
    fs.writeFileSync(path.join(packDir, name.replace('minecraft:', '') + '.json'), JSON.stringify(biome));
}
console.log(`datapack written to ${out} (${Object.keys(bundle.biomes).length} biomes)`);
