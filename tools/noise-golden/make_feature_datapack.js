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
// Block predicates: every type (recursively through predicate/predicates) must be in PORTED.predicates.
const blockPredicateOk = (p) => {
    if (!p || !PORTED.predicates.includes(p.type)) return false;
    if (p.type === "minecraft:all_of" || p.type === "minecraft:any_of") return p.predicates.every(blockPredicateOk);
    if (p.type === "minecraft:not") return blockPredicateOk(p.predicate);
    // would_survive: only the vegetation blocks of vegetation.rs (sapling and propagule paths are not in the list).
    if (p.type === "minecraft:would_survive") return PORTED.simple_blocks.includes(p.state.split("[")[0]);
    return true;
};
// simple_block provider: every block it can answer with is in PORTED.simple_blocks, rule_based predicates are ported.
const simpleBlockOk = (sp) => {
    if (typeof sp === "string") return PORTED.simple_blocks.includes(sp);
    if (sp.type === undefined) return PORTED.simple_blocks.includes(sp.id);
    if (sp.type === "minecraft:weighted") return sp.entries.every((e) => simpleBlockOk(e.data));
    if (sp.type === "minecraft:randomized_int") return simpleBlockOk(sp.source);
    if (sp.type === "minecraft:rule_based") {
        return sp.rules.every((r) => blockPredicateOk(r.if_true) && simpleBlockOk(r.then)) && (sp.fallback === undefined || simpleBlockOk(sp.fallback));
    }
    return false;
};
const placementOk = (list, top) =>
    (list || []).every(
        (m) =>
            PORTED.placements.includes(m.type) &&
            (top || m.type !== "minecraft:biome") &&
            (m.type !== "minecraft:block_predicate_filter" || blockPredicateOk(m.predicate))
    );
const featureOk = (ref) => {
    const cfg = typeof ref === "string" ? bundle.configured[ref] : ref;
    if (!cfg || !PORTED.types.includes(cfg.type)) return false;
    // Same rule as allow.rs: state providers are simple, weighted, randomized_int over a simple source,
    // or rule_based with ported predicates (recursively).
    const stateProviderOk = (sp) => {
        if (!sp || sp.type === undefined) return true;
        if (sp.type === "minecraft:weighted") return true;
        if (sp.type === "minecraft:randomized_int") return sp.source.type === undefined;
        if (sp.type === "minecraft:rule_based") {
            return (
                sp.rules.every((r) => blockPredicateOk(r.if_true) && stateProviderOk(r.then)) &&
                (sp.fallback === undefined || stateProviderOk(sp.fallback))
            );
        }
        return false;
    };
    if (cfg.type === "minecraft:block_column") {
        if (!cfg.layers.every((l) => stateProviderOk(l.provider)) || !blockPredicateOk(cfg.allowed_placement)) return false;
    }
    if (cfg.type === "minecraft:disk" && (!blockPredicateOk(cfg.target) || !stateProviderOk(cfg.state_provider))) return false;
    if (cfg.type === "minecraft:simple_block" && !simpleBlockOk(cfg.to_place)) return false;
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
// RINE_FEATURE_ALLOW_LIST: a file with one placed feature name per line replaces the rule (the full vanilla
// decoration golden lists every feature the Rust decoration compiles, see feature_golden.rs).
const ALLOW_LIST = process.env.RINE_FEATURE_ALLOW_LIST
    ? new Set(fs.readFileSync(process.env.RINE_FEATURE_ALLOW_LIST, 'utf8').split(String.fromCharCode(10)).map((l) => l.trim()).filter(Boolean))
    : null;
const allowed = (placedName) => (ALLOW_LIST ? ALLOW_LIST.has(placedName) : placedOk(bundle.placed[placedName], true));

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
