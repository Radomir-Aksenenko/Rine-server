// Drives a vanilla 26.3 server to generate a square of chunks with the feature datapack, then dumps the
// final block states with dump_mca_chunks.js.
//
//   node tools/noise-golden/run_feature_golden.js <runDir> <minCx> <minCz> <size> <outFile>
//
// <runDir> is created if missing: it gets eula.txt, server.properties (seed 1592598566, structures off,
// port 25566, offline), the libraries of vanilla-data/run, and the datapack in <runDir>/golden_world/datapacks.
// The server jar comes from vanilla-data/versions/26.3/server-26.3.jar (RINE_VANILLA_DIR overrides vanilla-data).
// Requires Java 25 on PATH.
'use strict';
const fs = require('fs');
const path = require('path');
const { spawn, execFileSync } = require('child_process');

const REPO = path.resolve(__dirname, '..', '..');
const VANILLA = process.env.RINE_VANILLA_DIR || path.join(REPO, 'vanilla-data');
const [runDir, minCx, minCz, size, outFile] = process.argv.slice(2);
if (!outFile) throw new Error('usage: run_feature_golden.js <runDir> <minCx> <minCz> <size> <outFile>');
const run = path.resolve(runDir);
const forceMin = Number(minCx) * 16;
const forceMax = (Number(minCx) + Number(size)) * 16 - 1;
const forceMinZ = Number(minCz) * 16;
const forceMaxZ = (Number(minCz) + Number(size)) * 16 - 1;

fs.mkdirSync(run, { recursive: true });
fs.copyFileSync(path.join(VANILLA, 'run', 'eula.txt'), path.join(run, 'eula.txt'));
fs.writeFileSync(
    path.join(run, 'server.properties'),
    [
        'level-seed=1592598566',
        'generate-structures=false',
        'level-name=golden_world',
        'server-port=25566',
        'online-mode=false',
        'enable-rcon=false',
        'enable-query=false',
        'enable-status=false',
        'spawn-protection=0',
        'view-distance=10',
        'simulation-distance=10',
        'max-players=2',
        'pause-when-empty-seconds=-1',
        'level-type=minecraft\\:normal',
        'motd=rine feature golden',
        '',
    ].join('\n')
);
const dpDir = path.join(run, 'golden_world', 'datapacks', 'rine_feature_golden');
execFileSync(process.execPath, [path.join(__dirname, 'make_feature_datapack.js'), dpDir], { stdio: 'inherit' });

// Classpath as in run_worldgen.sh: the server jar plus every library jar of vanilla-data/libraries.
const jars = [path.join(VANILLA, 'versions', '26.3', 'server-26.3.jar')];
const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        const p = path.join(dir, e.name);
        if (e.isDirectory()) walk(p);
        else if (p.endsWith('.jar')) jars.push(p);
    }
};
walk(path.join(VANILLA, 'libraries'));
const cp = [jars[0], ...jars.slice(1).sort()].join(path.delimiter);
const child = spawn('java', ['-cp', cp, 'net.minecraft.server.Main', 'nogui'], { cwd: run, stdio: ['pipe', 'pipe', 'pipe'] });
let log = '';
let stage = 'starting';
let idleTimer = null;
const send = (cmd) => child.stdin.write(cmd + '\n');
const armIdle = () => {
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
        if (stage === 'forced') {
            stage = 'saving';
            send('save-all flush');
        }
    }, 60000);
};
const onOutput = (chunk) => {
    const text = chunk.toString();
    log += text;
    process.stdout.write(text);
    if (stage === 'starting' && log.includes('Done (')) {
        stage = 'forced';
        // Freeze ticking first: scheduled fluid/block ticks, random ticks (grass, sugar cane, kelp, bamboo)
        // and weather are gated by TickRateManager.runsNormally(). Distance-manager updates still run, so
        // forceloaded chunks keep generating.
        send('tick freeze');
        send(`forceload add ${forceMin} ${forceMinZ} ${forceMax} ${forceMaxZ}`);
        armIdle();
    } else if (stage === 'forced') {
        armIdle();
    } else if (stage === 'saving' && log.includes('Saved the game')) {
        stage = 'stopping';
        send('stop');
    }
};
child.stdout.on('data', onOutput);
child.stderr.on('data', (c) => process.stderr.write(c));
child.on('exit', (code) => {
    clearTimeout(idleTimer);
    if (stage !== 'stopping') {
        console.error('server exited early, code', code);
        process.exit(1);
    }
    execFileSync(
        process.execPath,
        [
            path.join(__dirname, 'dump_mca_chunks.js'),
            path.join(run, 'golden_world', 'dimensions', 'minecraft', 'overworld', 'region'),
            minCx,
            minCz,
            size,
            outFile,
        ],
        { stdio: 'inherit' }
    );
});
