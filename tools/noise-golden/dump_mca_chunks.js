// Reads final block states of a square of chunks from a vanilla region directory (*.mca) and writes the
// binary golden consumed by crates/rine-worldgen/tests/feature_golden.rs.
//
// Output format (little endian):
//   u32 chunk count
//   per chunk: i32 cx, i32 cz, u32 palette count, palette entries (u16 byte length + UTF-8 canonical
//              state "name[k=v,...]" with keys sorted), then (height * 256) u16 palette indices in
//              vanilla order ((y - minY) * 256 + z * 16 + x).
// Chunks that are missing from the region files are reported and abort the dump.
//
// Usage: node tools/noise-golden/dump_mca_chunks.js <regionDir> <minCx> <minCz> <size> <outFile>
'use strict';
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const [regionDir, minCxArg, minCzArg, sizeArg, outFile] = process.argv.slice(2);
if (!outFile) throw new Error('usage: dump_mca_chunks.js <regionDir> <minCx> <minCz> <size> <outFile>');
const minCx = Number(minCxArg);
const minCz = Number(minCzArg);
const size = Number(sizeArg);
const MIN_Y = -64;
const HEIGHT = 384;

// --- minimal NBT reader -------------------------------------------------------------------------
function readNbt(buf) {
    let pos = 0;
    const u8 = () => buf.readUInt8(pos++);
    const i16 = () => {
        const v = buf.readInt16BE(pos);
        pos += 2;
        return v;
    };
    const i32 = () => {
        const v = buf.readInt32BE(pos);
        pos += 4;
        return v;
    };
    const i64 = () => {
        const v = buf.readBigInt64BE(pos);
        pos += 8;
        return v;
    };
    const str = () => {
        const n = buf.readUInt16BE(pos);
        pos += 2;
        const s = buf.toString('utf8', pos, pos + n);
        pos += n;
        return s;
    };
    function payload(type) {
        switch (type) {
            case 1:
                return u8() << 24 >> 24;
            case 2:
                return i16();
            case 3:
                return i32();
            case 4:
                return i64();
            case 5:
                pos += 4;
                return null;
            case 6:
                pos += 8;
                return null;
            case 7: {
                const n = i32();
                pos += n;
                return null;
            }
            case 8:
                return str();
            case 9: {
                const et = u8();
                const n = i32();
                const arr = [];
                for (let i = 0; i < n; i++) arr.push(payload(et));
                return arr;
            }
            case 10: {
                const obj = {};
                for (;;) {
                    const t = u8();
                    if (t === 0) break;
                    const name = str();
                    obj[name] = payload(t);
                }
                return obj;
            }
            case 11: {
                const n = i32();
                const arr = new Array(n);
                for (let i = 0; i < n; i++) arr[i] = i32();
                return arr;
            }
            case 12: {
                const n = i32();
                const arr = new Array(n);
                for (let i = 0; i < n; i++) arr[i] = i64();
                return arr;
            }
            default:
                throw new Error('bad NBT tag ' + type);
        }
    }
    const rootType = u8();
    str();
    return payload(rootType);
}

// --- region file access -------------------------------------------------------------------------
function readChunk(regionBuf, cx, cz) {
    const idx = (cx & 31) + (cz & 31) * 32;
    const entry = regionBuf.readUInt32BE(idx * 4);
    const sector = entry >>> 8;
    if (sector === 0) return null;
    const off = sector * 4096;
    const length = regionBuf.readUInt32BE(off);
    const type = regionBuf.readUInt8(off + 4);
    const data = regionBuf.subarray(off + 5, off + 4 + length);
    const raw = type === 2 ? zlib.inflateSync(data) : type === 1 ? zlib.gunzipSync(data) : null;
    if (!raw) throw new Error('unsupported chunk compression ' + type);
    return readNbt(raw);
}

// --- block states -------------------------------------------------------------------------------
function canonical(entry) {
    // 26.3 stores plain states as bare names; states with properties as {Name, Properties}.
    if (typeof entry === 'string') return entry;
    const props = entry.Properties || {};
    const keys = Object.keys(props).sort();
    if (keys.length === 0) return entry.Name;
    return entry.Name + '[' + keys.map((k) => k + '=' + props[k]).join(',') + ']';
}

function sectionBlocks(section) {
    const bs = section.block_states;
    const palette = bs.palette.map(canonical);
    const out = new Uint16Array(4096);
    if (palette.length === 1) return { palette, indices: out };
    const bits = Math.max(4, Math.ceil(Math.log2(palette.length)));
    const perLong = Math.floor(64 / bits);
    const mask = (1n << BigInt(bits)) - 1n;
    const data = bs.data;
    for (let i = 0; i < 4096; i++) {
        const li = Math.floor(i / perLong);
        const shift = BigInt((i % perLong) * bits);
        const word = BigInt.asUintN(64, BigInt(data[li]));
        out[i] = Number((word >> shift) & mask);
    }
    return { palette, indices: out };
}

function chunkBlocks(nbt) {
    const palette = new Map(); // canonical -> index
    const names = [];
    const blocks = new Uint16Array(HEIGHT * 256);
    const intern = (s) => {
        if (!palette.has(s)) {
            palette.set(s, names.length);
            names.push(s);
        }
        return palette.get(s);
    };
    for (const section of nbt.sections || []) {
        const y0 = section.Y * 16;
        if (y0 < MIN_Y || y0 >= MIN_Y + HEIGHT) continue;
        const { palette: secPal, indices } = sectionBlocks(section);
        const map = secPal.map(intern);
        for (let i = 0; i < 4096; i++) {
            // section order is y, z, x: index = (y * 16 + z) * 16 + x
            const ly = i >> 8;
            const z = (i >> 4) & 15;
            const x = i & 15;
            blocks[(y0 - MIN_Y + ly) * 256 + z * 16 + x] = map[indices[i]];
        }
    }
    return { names, blocks };
}

// --- main ---------------------------------------------------------------------------------------
const regionFiles = new Map();
const regionOf = (cx, cz) => {
    const rx = cx >> 5;
    const rz = cz >> 5;
    const key = rx + ',' + rz;
    if (!regionFiles.has(key)) {
        const file = path.join(regionDir, `r.${rx}.${rz}.mca`);
        regionFiles.set(key, fs.readFileSync(file));
    }
    return regionFiles.get(key);
};

const parts = [];
const header = Buffer.alloc(4);
header.writeUInt32LE(size * size, 0);
parts.push(header);
for (let dz = 0; dz < size; dz++) {
    for (let dx = 0; dx < size; dx++) {
        const cx = minCx + dx;
        const cz = minCz + dz;
        const nbt = readChunk(regionOf(cx, cz), cx, cz);
        if (!nbt) throw new Error(`chunk ${cx},${cz} missing in region files`);
        const { names, blocks } = chunkBlocks(nbt);
        const head = Buffer.alloc(12);
        head.writeInt32LE(cx, 0);
        head.writeInt32LE(cz, 4);
        head.writeUInt32LE(names.length, 8);
        parts.push(head);
        for (const n of names) {
            const b = Buffer.from(n, 'utf8');
            const l = Buffer.alloc(2);
            l.writeUInt16LE(b.length, 0);
            parts.push(l, b);
        }
        parts.push(Buffer.from(blocks.buffer));
    }
}
fs.writeFileSync(outFile, Buffer.concat(parts));
console.log(`wrote ${size * size} chunks to ${outFile}`);
