// Usage: node tools/noise-golden/bundle_feature_templates.js <vanilla-run> <minecraft-data> [output]
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const [vanillaRun, minecraftData, output = 'crates/rine-worldgen/assets/worldgen/feature_templates.json'] = process.argv.slice(2);
if (!vanillaRun || !minecraftData) {
    throw new Error('Expected the vanilla run and data/minecraft directories');
}

const libraries = path.resolve(vanillaRun, 'libraries');
const jars = fs.readdirSync(libraries, { recursive: true })
    .filter(name => name.endsWith('.jar'))
    .sort()
    .map(name => path.join(libraries, name));
jars.unshift(path.resolve(vanillaRun, 'versions/26.3/server-26.3.jar'));
const result = spawnSync('java', [
    '-cp', jars.join(path.delimiter),
    path.join(__dirname, 'BundleFeatureTemplates.java'),
    path.resolve(minecraftData, 'structure'),
    path.resolve(minecraftData, 'worldgen/processor_list'),
    path.resolve(output),
], { stdio: 'inherit' });
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
