use super::block_entity::BlockEntityData;
use super::region::{Pos, Region};
use super::rng::WorldgenRng;
use super::state::Palette;

const HORIZONTAL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
const MOBS: [&str; 4] = [
    "minecraft:skeleton",
    "minecraft:zombie",
    "minecraft:zombie",
    "minecraft:spider",
];

pub(super) struct MonsterRoom {
    protected: u16,
    air: u16,
    stone: u16,
    mossy: u16,
    chest_block: u16,
    chests: [u16; 4],
    spawner: u16,
}

impl MonsterRoom {
    pub fn compile(pal: &mut Palette, tags: &[String]) -> Result<Self, String> {
        Ok(Self {
            protected: tags
                .binary_search_by(|s| s.as_str().cmp("minecraft:features_cannot_replace"))
                .map_err(|_| "missing features_cannot_replace tag")? as u16,
            air: pal.default_state("minecraft:cave_air"),
            stone: pal.default_state("minecraft:cobblestone"),
            mossy: pal.default_state("minecraft:mossy_cobblestone"),
            chest_block: pal.block("minecraft:chest"),
            chests: ["north", "east", "south", "west"]
                .map(|face| pal.state_from_parts("minecraft:chest", &[("facing", face)])),
            spawner: pal.default_state("minecraft:spawner"),
        })
    }

    fn safe_set(&self, region: &mut Region, pos: Pos, old: u16, state: u16) {
        if !region.tags.has(old, self.protected) {
            region.write(pos, state, 2);
        }
    }

    fn chest_state(&self, region: &Region, (x, y, z): Pos) -> u16 {
        // StructurePiece.reorient
        let at = |dir: usize| {
            let (dx, dz) = HORIZONTAL[dir];
            region.get_block(x + dx, y, z + dz)
        };
        let mut solid = None;
        for dir in 0..4 {
            let state = at(dir);
            if region.tags.is_block(state, self.chest_block) {
                return self.chests[0];
            }
            if region.tags.solid_render(state) {
                if solid.is_some() {
                    solid = None;
                    break;
                }
                solid = Some(dir);
            }
        }
        if let Some(dir) = solid {
            return self.chests[(dir + 2) % 4];
        }
        let mut facing = 0;
        for shift in [2, 1, 2] {
            if region.tags.solid_render(at(facing)) {
                facing = (facing + shift) % 4;
            }
        }
        self.chests[facing]
    }

    pub fn place(&self, region: &mut Region, rng: &mut WorldgenRng, (x, y, z): Pos) -> bool {
        let xr = rng.next_int_bound(2) + 2;
        let zr = rng.next_int_bound(2) + 2;
        let (ex, ez) = (xr + 1, zr + 1);
        let mut holes = 0;
        for dx in -ex..=ex {
            for dy in -1..=4 {
                for dz in -ez..=ez {
                    let (px, py, pz) = (x + dx, y + dy, z + dz);
                    let state = region.get_block(px, py, pz);
                    if (dy == -1 || dy == 4) && !region.tags.solid(state) {
                        return false;
                    }
                    if dy == 0
                        && (dx.abs() == ex || dz.abs() == ez)
                        && region.tags.is_air(state)
                        && region.is_air(px, py + 1, pz)
                    {
                        holes += 1;
                    }
                }
            }
        }
        if !(1..=5).contains(&holes) {
            return false;
        }
        for dx in -ex..=ex {
            for dy in (-1..=3).rev() {
                for dz in -ez..=ez {
                    let p = (x + dx, y + dy, z + dz);
                    let wall = region.get_block(p.0, p.1, p.2);
                    if dy == -1 || dx.abs() == ex || dz.abs() == ez {
                        if p.1 >= region.min_y()
                            && !region.tags.solid(region.get_block(p.0, p.1 - 1, p.2))
                        {
                            region.write(p, self.air, 2);
                        } else if region.tags.solid(wall)
                            && !region.tags.is_block(wall, self.chest_block)
                        {
                            let state = if dy == -1 && rng.next_int_bound(4) != 0 {
                                self.mossy
                            } else {
                                self.stone
                            };
                            self.safe_set(region, p, wall, state);
                        }
                    } else if !region.tags.is_block(wall, self.chest_block) && wall != self.spawner
                    {
                        self.safe_set(region, p, wall, self.air);
                    }
                }
            }
        }
        for _ in 0..2 {
            for _ in 0..3 {
                let px = x + rng.next_int_bound(xr * 2 + 1) - xr;
                let pz = z + rng.next_int_bound(zr * 2 + 1) - zr;
                let old = region.get_block(px, y, pz);
                if !region.tags.is_air(old) {
                    continue;
                }
                let walls = HORIZONTAL
                    .iter()
                    .filter(|&&(dx, dz)| region.tags.solid(region.get_block(px + dx, y, pz + dz)))
                    .count();
                if walls == 1 {
                    let pos = (px, y, pz);
                    self.safe_set(region, pos, old, self.chest_state(region, pos));
                    if region
                        .tags
                        .is_block(region.get_block(px, y, pz), self.chest_block)
                    {
                        region.record_block_entity(
                            pos,
                            "minecraft:chest",
                            BlockEntityData::Loot {
                                table: "minecraft:chests/simple_dungeon".into(),
                                seed: rng.next_long(),
                            },
                        );
                    }
                    break;
                }
            }
        }
        self.safe_set(region, (x, y, z), region.get_block(x, y, z), self.spawner);
        if region.get_block(x, y, z) == self.spawner {
            let entity = MOBS[rng.next_int_bound(MOBS.len() as i32) as usize];
            region.record_block_entity(
                (x, y, z),
                "minecraft:mob_spawner",
                BlockEntityData::Spawner {
                    entity: entity.into(),
                },
            );
        }
        true
    }
}
