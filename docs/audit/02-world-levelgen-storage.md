# Аудит: ванильный world/level (levelgen, storage, chunk и прочее)

Дата: 2026-11-15. Только чтение кода, ничего не менялось.

Охват: папка `Paper-26.3/paper-server/src/minecraft/java/net/minecraft/world/level`, кроме подпапок `block`, `redstone`, `material`, `gameevent` (их смотрит другой агент). Итого файлов: 907, из них 60 файлов `package-info`.

## Сводка

Считаю файлы, а не строки таблицы. Каждый файл стоит ровно в одной строке таблицы.

| Статус | Файлов |
|---|---|
| ЕСТЬ | 62 |
| ЧАСТИЧНО | 234 |
| НЕТ | 439 |
| не уверен | 22 |
| служебные | 150 |
| Всего | 907 |

Что важно:

- Генерация идёт в два слоя. Живой путь (`rine-world/src/gen/mod.rs`) берёт из `rine-worldgen` рельеф, аквифер, поверхность и карту биомов. Пещеры, фичи (деревья, руды, декор) и размещение там не участвуют. Деревья, руды и цветы считаются своим кодом. Всё портированное в `rine-worldgen` и сверенное с ванилью (фичи, размещение, карверы, украшения деревьев) в сервер не подключено.
- Структур нет совсем: деревень, крепостей, шахт, храмов, порталов, Незера и Энда. Это главный пробел этой части ванили. `locate.rs` говорит об этом прямо.
- Сохранение мира: свой формат RIN1 (LZ4 по регионам), скорборд в `data/scoreboard.dat`, состояние датапаков в `rine_datapacks.json`. Anvil и `level.dat` нет. Сущности не сохраняются. Время, погода, граница и точка спавна в файлы не пишутся.
- Граница мира хранит состояние и шлёт пакеты, но не наносит урон и не ограничивает движение.
- Из 59 ванильных правил игры 58 объявлены, но код читает только 11. Остальные 47 на игру не влияют.
- Таблицы добычи: только блоки, без сундуков и без зачарований. Неизвестная функция в таблице ничего не делает, неизвестное условие всегда ложно, неизвестное число даёт 1. Это тихие отказы, а не ошибки.

## Статусы

- **ЕСТЬ**: работает в живом сервере (с оговорками, если они указаны в строке).
- **ЧАСТИЧНО**: часть механики есть. Сюда входит код, портированный в `rine-worldgen` и сверенный с ванилью, но не подключённый к серверу, а также свои реализации вместо ванильных.
- **НЕТ**: механики нет или она только объявлена.
- **не уверен**: нужна ручная проверка, причина в строке.
- **служебные**: интерфейсы, базовые классы, реестры, `package-info`. Не оцениваются.

Пути Rine указаны от корня репозитория.

## levelgen (корень папки)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| Aquifer | Корень рельефа: аквифер, вода и лава под землёй | ЕСТЬ | crates/rine-worldgen/src/overworld/aquifer.rs. Идёт в живом пути Overworld::generate |
| Beardifier | Добавка к рельефу от структур (beardifier) | ЧАСТИЧНО | density/build.rs: тип beardifier заменён константой 0. Без структур это ожидаемо, но структур нет вовсе |
| BelowZeroRetrogen | Перевод старых миров на новую высоту (below zero retrogen) | НЕТ | Не портирован. Новым мирам не нужен |
| DebugLevelSource, FlatLevelSource | Плоский и отладочный мир | НЕТ | Не реализованы. Есть только обычный генератор |
| NoiseBasedChunkGenerator, NoiseChunk | Генерация чанка по шуму, NoiseChunk | ЧАСТИЧНО | crates/rine-world/src/gen/mod.rs (Generator) вызывает rine_worldgen::Overworld::generate. Своя обвязка: нет статусов чанка, структур и карверов |
| NoiseGeneratorSettings, NoiseRouter, NoiseRouterData, NoiseSettings, Noises, OverworldFunctionSet, RandomState | Настройки и наборы шумов и функций плотности | ЧАСТИЧНО | crates/rine-worldgen/src/density/ (build.rs, assets.rs). Только overworld: наборов для nether и end нет |
| Heightmap | Карты высот (heightmap) | ЧАСТИЧНО | crates/rine-world/src/chunk.rs: 3 из 6 типов (MOTION_BLOCKING, WORLD_SURFACE, MOTION_BLOCKING_NO_LEAVES). Нет OCEAN_FLOOR, OCEAN_FLOOR_WG, WORLD_SURFACE_WG |
| GenerationStep | Шаги декорации (GenerationStep) | ЧАСТИЧНО | crates/rine-worldgen/src/feature/mod.rs хранит только число шагов (DECORATION_STEPS = 11) |
| GeodeBlockSettings, GeodeCrackSettings, GeodeLayerSettings | Геоды: настройки слоёв, трещин и блоков | НЕТ | Геода не портирована |
| PatrolSpawner, PhantomSpawner | Спавн патрулей и фантомов | НЕТ | Мобов и спавна нет. Правила spawn_patrols и spawn_phantoms только объявлены в crates/rine-server/src/game/cmd/rules.rs |
| WorldDimensions, WorldGenSettings, WorldOptions | Набор измерений и опции генерации мира | НЕТ | crates/rine-server/src/config.rs знает только seed (по умолчанию 0x5EED_2026) и папку мира |
| NoiseSpawnFinder, SpawnTargetPoint | Поиск точки спавна | ЧАСТИЧНО | Свой find_spawn в crates/rine-world/src/gen/mod.rs. С ванильным алгоритмом не сверял, не уверен, что совпадает |
| VerticalAnchor | Якоря высоты: absolute, above_bottom, below_top | ЧАСТИЧНО | crates/rine-worldgen/src/feature/placement.rs (parse_anchor). Нужны только портированным фичам, в сервере не подключено |
| WorldGenerationContext | Границы генерации по высоте (WorldGenerationContext) | ЧАСТИЧНО | Свой параметр gen_depth и min_gen_y в rine-worldgen. Отдельного класса нет |
| XoroshiroRandomSource, Xoroshiro128PlusPlus, LegacyRandomSource, PositionalRandomFactory, WorldgenRandom | Генераторы случайных чисел: Xoroshiro, legacy, позиционный (сид из координат), WorldgenRandom | ЕСТЬ | crates/rine-worldgen/src/random.rs (Legacy, Positional, Xoroshiro), feature/rng.rs (WorldgenRng). Шумы сверены с jar, 0 расхождений (docs/HANDOFF.md) |
| Column, MarsagliaPolarGaussian, BitRandomSource, SingleThreadedRandomSource, ThreadSafeLegacyRandomSource | Column, Gaussian, BitRandomSource, SingleThreadedRandomSource, ThreadSafeLegacyRandomSource | не уверен | Классов с такими именами в Rine нет. Не понял, нужны ли они рельефу (Column и Gaussian могут нужны фичам). Проверить вручную |
| Density, RandomSupport | Интерфейс Density, утилиты рандома | служебные | Служебное |

## levelgen/synth (шумы)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| PerlinNoise, SimplexNoise, NormalNoise, SmearedPerlinNoise, BlendedNoise, NoiseStack | Шумы: Perlin, Simplex, Normal, smeared Perlin, blended noise, NoiseStack | ЕСТЬ | crates/rine-worldgen/src/noise.rs (реэкспорт в lib.rs). Живой путь рельефа |
| GradientNoise, LegacyFbmInitializer, Noise, NoiseUtils | GradientNoise, LegacyFbmInitializer, Noise, NoiseUtils | не уверен | Отдельных классов с такими именами нет. Возможно, логика внутри noise.rs. Проверить вручную |

## levelgen/densityfunction (функции плотности)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| CacheId, CachingDensitySampler, ContextBoundSampler, DensityBuffer, DensityBufferArena, DensityBufferPool, DensityFunction, DensityFunctionCompiler, DensityFunctions, DensitySampler, DensitySamplerSet, DensityVolume, DfRewriteRule, DistanceMetric, SamplerContext, ScopedDensityBuffer, TilingMode | Граф функций плотности, сборка и вычисление | ЧАСТИЧНО | Граф: crates/rine-worldgen/src/density/tree.rs, сборка из JSON: density/build.rs, объём: density/volume.rs. Буферы и сэмплеры устроены иначе, аналогов по именам нет |
| ConstantFunction, NoiseFunction, ShiftNoiseFunction, GradientFunction | Простые функции: константа, шум, сдвиг, градиент | ЕСТЬ | crates/rine-worldgen/src/density/build.rs (constant, noise, shift, shift_a, shift_b, gradient) |
| SimpleDensityFunction | Базовый интерфейс простых функций плотности | служебные | Служебное |
| DistanceToPointFunction, EndIslandFunction | Расстояние до точки, острова Энда | НЕТ | В build.rs таких типов нет. Встреча такого типа в данных даст ошибку |
| BinaryFunction, UnaryFunction, ClampFunction, CacheFunction, FindTopSurfaceFunction, InterpolatedFunction, IntervalSelectFunction, LerpFunction, RangeChoiceFunction, SplineFunction | Операции: сложение и т.п., abs, clamp, cache, сплайн, lerp, interval_select, range_choice, find_top_surface, interpolated | ЕСТЬ | crates/rine-worldgen/src/density/build.rs, density/spline.rs |
| BlendDensityFunction | Смешивание с прошлыми чанками (blend_density) | ЧАСТИЧНО | В build.rs заглушка. Смешивания нет |
| PowFunction, SliceFunction, RoundFunction | Возведение в степень, срез, округление | НЕТ | В build.rs таких типов нет |

## levelgen/material (правила поверхности)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| MaterialRules, MaterialSystem, MaterialRuleContext, BandlandsRule, BlockRule, ConditionRule, OreVeinRule, SequenceRule | Правила поверхности: MaterialRules, MaterialSystem, MaterialRuleContext; правила block, bandlands, sequence, condition, ore_vein | ЕСТЬ | crates/rine-worldgen/src/overworld/rules.rs (5 типов правил), overworld/surface.rs. Живой путь |
| AbovePreliminarySurfaceCondition, BiomeCondition, HoleCondition, NoiseThresholdCondition, NotCondition, SteepCondition, StoneDepthCondition, TemperatureCondition, VerticalGradientCondition, WaterCondition, YCondition | Условия правил поверхности: biome, noise_threshold, vertical_gradient, y_above, water, temperature, steep, hole, above_preliminary_surface, stone_depth, not | ЕСТЬ | crates/rine-worldgen/src/overworld/rules.rs (11 условий) |
| MaterialRule, RuleEvaluator, ConditionEvaluator, MaterialCondition | Интерфейсы и оценщики правил поверхности | служебные | Служебное |

## levelgen/feature (конфигурированные фичи)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| OreFeature, AbstractOreFeature, DiskFeature, SpringFeature, BlockColumnFeature, BlueIceFeature, SimpleBlockFeature, TreeFeature, FallenTreeFeature, RandomBooleanSelectorFeature, SimpleRandomSelectorFeature, WeightedRandomSelectorFeature, SequenceFeature, WeightedPlacedFeature, FeatureTypes | Портированные фичи: руды, диски, родники, лёд, колонны, простой блок, деревья, поваленные деревья, селекторы, sequence | ЧАСТИЧНО | crates/rine-worldgen/src/feature/ (ore.rs, disk.rs, spring.rs, ice.rs, column.rs, simple_block.rs, tree/, mod.rs). Сверены с ванилью по golden (руды около 0,01% расхождений, деревья около 0,03%). В сервер не подключены. Живой путь использует свои деревья и руды из crates/rine-world/src/gen/trees.rs и ores.rs |
| Feature, FeatureCountTracker, BlockReplacement | Базовый класс фичи и вспомогательные классы | служебные | Служебное |
| AbstractHugeMushroomFeature, BambooFeature, BlockBlobFeature, BlockPileFeature, BonusChestFeature, ChorusPlantFeature, CoralClawFeature, CoralTreeFeature, CuboidPlacement, DeltaFeature, EndGatewayFeature, EndIslandFeature, EndPlatformFeature, EndPodiumFeature, EndSpikeFeature, FillLayerFeature, FossilFeature, GeodeFeature, HugeBrownMushroomFeature, HugeFungusFeature, HugeRedMushroomFeature, IcebergFeature, LakeFeature, LargeDripstoneFeature, MonsterRoomFeature, MultifaceGrowthFeature, NoOpFeature, OverlayFeature, ProjectedRandomPatchySquare, RandomNeighborSpreadFeature, RandomSelectorFeature, ReplaceBlobsFeature, ReplaceBlockFeature, RootSystemFeature, ScatteredOreFeature, SculkPatchFeature, SingleBlockPillarFeature, SnowAndFreezeFeature, SpeleothemClusterFeature, SpeleothemFeature, SpeleothemUtils, SpikeFeature, SteppedColumnClusterFeature, TemplateFeature, UnderwaterMagmaFeature, VegetationPatchFeature, VinesFeature, VoidStartPlatformFeature, WaterloggedVegetationPatchFeature | Все прочие фичи: бамбук, блоб-фичи, бонусный сундук, кораллы, фичи Энда, геоды, огромные грибы, айсберги, озёра, капли, сталактиты, вайны, пещерные фичи, мох, споры и т.п. | НЕТ | Не портированы. rine-worldgen падает с ошибкой на неизвестном типе (feature/mod.rs), поэтому такие фичи не работают нигде. Живой декор (трава, цветы, кактусы, тростник, тыквы, мёртвые кусты) сделан отдельно в crates/rine-world/src/gen/decor.rs, каменные пятна (blob) в gen/ores.rs, и оба работают без фич |
| ThreeLayersFeatureSize, TwoLayersFeatureSize | Размер ствола: два и три слоя | ЧАСТИЧНО | crates/rine-worldgen/src/feature/tree/mod.rs (two_layers_feature_size, three_layers_feature_size). Не подключено |
| FeatureSize, FeatureSizeType | Базовый тип размера ствола | служебные | Служебное |
| AcaciaFoliagePlacer, BlobFoliagePlacer, BushFoliagePlacer, CherryFoliagePlacer, DarkOakFoliagePlacer, FancyFoliagePlacer, MegaJungleFoliagePlacer, MegaPineFoliagePlacer, PineFoliagePlacer, RandomSpreadFoliagePlacer, SpruceFoliagePlacer | Формы кроны: blob, bush, fancy, spruce, pine, acacia, dark oak, mega pine, mega jungle, random spread, cherry | ЧАСТИЧНО | crates/rine-worldgen/src/feature/tree/foliage.rs (11 типов). Не подключено к серверу |
| PoplarFoliagePlacer | Крона тополя | НЕТ | В foliage.rs нет poplar_foliage_placer |
| FoliagePlacer, FoliagePlacerType | Базовые классы крон | служебные | Служебное |
| MangroveRootPlacer, MangroveRootPlacement | Корни мангров | ЧАСТИЧНО | crates/rine-worldgen/src/feature/tree/root.rs: только mangrove_root_placer |
| AboveRootPlacement | Корни над поверхностью (above_root) | не уверен | Не понял, есть ли в root.rs. Проверить вручную |
| RootPlacer, RootPlacerType | Базовые классы корней | служебные | Служебное |
| CherryTrunkPlacer, DarkOakTrunkPlacer, FancyTrunkPlacer, ForkingTrunkPlacer, GiantTrunkPlacer, MegaJungleTrunkPlacer, StraightTrunkPlacer, UpwardsBranchingTrunkPlacer | Формы ствола: cherry, dark oak, fancy, forking, giant, mega jungle, straight, upwards branching | ЧАСТИЧНО | crates/rine-worldgen/src/feature/tree/trunk.rs (8 типов). Не подключено к серверу |
| BendingTrunkPlacer, PoplarTrunkPlacer | Изогнутый ствол и ствол тополя | НЕТ | В trunk.rs нет bending_trunk_placer и poplar_trunk_placer |
| TrunkPlacer, TrunkPlacerType | Базовые классы стволов | служебные | Служебное |
| SimpleStateProvider, WeightedStateProvider, RandomizedIntStateProvider, RuleBasedStateProvider | Провайдеры состояния блока: simple, weighted, randomized_int, rule_based | ЧАСТИЧНО | crates/rine-worldgen/src/feature/state_provider.rs (4 типа из 10). Не подключено |
| CopyPropertiesProvider, DualNoiseProvider, NoiseBasedStateProvider, NoiseProvider, NoiseThresholdProvider, RandomBlockProvider, RotatedBlockProvider | Провайдеры с шумом, копией свойств, поворотом, случайным блоком | НЕТ | Не реализованы: copy_properties, dual_noise, noise, noise_threshold, rotated, random_block |
| BlockStateProvider, BlockStateProviderTypes | Базовые классы провайдеров состояний | служебные | Служебное |
| AlterGroundDecorator, AttachedToLeavesDecorator, AttachedToLogsDecorator, BeehiveDecorator, CocoaDecorator, CreakingHeartDecorator, LeaveVineDecorator, PaleMossDecorator, PlaceOnGroundDecorator, TrunkVineDecorator | Украшения деревьев: лозы, какао, ульи, прикреплённые блоки, мох, креакинг-сердце и др. | ЧАСТИЧНО | crates/rine-worldgen/src/feature/tree/decorator.rs (10 типов). Не подключено |
| ShelfMushroomDecorator | Декоратор грибной полки | НЕТ | В decorator.rs нет shelf_mushroom |
| TreeDecorator, TreeDecoratorType | Базовые классы декораторов | служебные | Служебное |

## levelgen/placement (модификаторы размещения)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| BiomeFilter, BlockPredicateFilter, CountPlacement, HeightRangePlacement, HeightmapPlacement, InSquarePlacement, NoiseBasedCountPlacement, NoiseThresholdCountPlacement, OffsetPlacement, RarityFilter, RepeatingPlacement | Модификаторы размещения: count, rarity, in_square, height_range, biome, block_predicate_filter, offset, heightmap, noise_threshold_count, noise_based_count | ЧАСТИЧНО | crates/rine-worldgen/src/feature/placement.rs (10 типов). Используются только портированными фичами, которые в сервере не запускаются |
| FeaturePlacer, PlacedFeature, PlacementModifierTypes | Запуск размещённой фичи и реестр модификаторов | ЧАСТИЧНО | feature/mod.rs (placed_from_parts), feature/placement.rs. Реестр покрывает 10 из 17 типов |
| CaveSurface, CountOnEveryLayerPlacement, EnvironmentScanPlacement, FixedPlacement, RandomChancePlacement, RandomlySelectedPlacement, SurfaceRelativeThresholdFilter, SurfaceWaterDepthFilter | Слой поверхности, сканирование среды, случайный шанс, фильтры поверхности и глубины воды, фиксированное место | НЕТ | Не реализованы: count_on_every_layer, environment_scan, random_chance, randomly_selected, surface_relative_threshold_filter, surface_water_depth_filter, fixed_placement. Enum CaveSurface тоже не портирован |
| PlacementContext, PlacementFilter, PlacementModifier | Интерфейсы и контексты размещения | служебные | Служебное |

## levelgen/structure (структуры)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| BoundingBox, BuiltinStructureSets, BuiltinStructures, PoolElementStructurePiece, PostPlacementProcessor, ScatteredFeaturePiece, SinglePieceStructure, Structure, StructureCheck, StructureCheckResult, StructurePiece, StructureSet, StructureSpawnOverride, StructureStart, StructureType, TemplateStructurePiece, TerrainAdjustment | Базовые классы структур, ограничивающие боксы, кусочки и генерация структур по ландшафту | НЕТ | В Rine структур нет. crates/rine-server/src/game/cmd/commands/misc/locate.rs прямо пишет: структуры и POI не генерируются. Команда /place structure не работает |
| BuriedTreasurePieces, BuriedTreasureStructure, DesertPyramidPiece, DesertPyramidStructure, EndCityPieces, EndCityStructure, IglooPieces, IglooStructure, JigsawStructure, JungleTemplePiece, JungleTempleStructure, MineshaftPieces, MineshaftStructure, NetherFortressPieces, NetherFortressStructure, NetherFossilPieces, NetherFossilStructure, OceanMonumentPieces, OceanMonumentStructure, OceanRuinPieces, OceanRuinStructure, RuinedPortalPiece, RuinedPortalStructure, ShipwreckPieces, ShipwreckStructure, StrongholdPieces, StrongholdStructure, SwampHutPiece, SwampHutStructure, WoodlandMansionPieces, WoodlandMansionStructure | Сами структуры: деревни (через JigsawStructure), крепость, шахты, храмы пустыни и джунглей, хижина ведьмы, иглу, особняк, Незер (крепость, окаменелости), Энд (город), океанский памятник и руины, корабль, погребённый клад, руины портала, стронгхолд | НЕТ | Не реализованы. Деревни, крепости, шахты, порталы и остальные структуры в мире не появляются |
| PiecesContainer, StructurePieceSerializationContext, StructurePieceType, StructurePiecesBuilder | Кусочки структур (pieces) | НЕТ | Не реализованы |
| AbstractSpreadingStructurePlacement, ConcentricRingsStructurePlacement, DimensionOriginStructurePlacement, RandomSpreadStructurePlacement, RandomSpreadType, StructurePlacement, StructurePlacements | Размещение структур: случайное распределение, кольца (стронгхолды), смещение от измерения | НЕТ | Не реализовано |
| DimensionPadding, EmptyPoolElement, FeaturePoolElement, JigsawJunction, JigsawPlacement, LegacySinglePoolElement, ListPoolElement, SinglePoolElement, StructurePoolElement, StructurePoolElementType, StructureTemplatePool, DirectPoolAlias, PoolAliasBinding, PoolAliasBindings, PoolAliasLookup, RandomGroupPoolAlias, RandomPoolAlias | Пулы джигсо (pools): основа деревень, бастионов, трейлов | НЕТ | Не реализованы. Файлы пулов в jar не разбираются |
| AllOfRuleTest, AlwaysTrueTest, AnyOfRuleTest, AxisAlignedLinearPosTest, BlackstoneReplaceProcessor, BlockAgeProcessor, BlockIgnoreProcessor, BlockMatchTest, BlockRotProcessor, BlockStateMatchTest, CappedProcessor, GravityProcessor, HeightMatchTest, JigsawReplacementProcessor, LavaSubmergedBlockProcessor, LinearPosTest, LiquidSettings, NopProcessor, NotRuleTest, PosAlwaysTrueTest, PosRuleTest, PosRuleTestType, ProcessorRule, ProtectedBlockProcessor, RandomBlockMatchTest, RandomBlockStateMatchTest, RuleProcessor, RuleTest, RuleTestType, StructurePlaceSettings, StructureProcessor, StructureProcessorList, StructureProcessorType, StructureProcessorTypes, StructureTemplate, StructureTemplateManager, TagMatchTest, DirectoryTemplateSource, ResourceManagerTemplateSource, TemplatePathFactory, TemplateSource, AppendLoot, AppendStatic, Clear, Passthrough, RuleBlockEntityModifier, RuleBlockEntityModifierType | Шаблоны структур (templatesystem): NBT-шаблоны, процессоры, правила блоков | НЕТ | Не реализованы. crates/rine-server/src/game/cmd/commands/misc/place.rs возвращает template.invalid, потому что файлов шаблонов нет |

## levelgen/blockpredicates и levelgen/heightproviders

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| AllOfPredicate, AnyOfPredicate, NotPredicate, TrueBlockPredicate, MatchingBlocksPredicate, MatchingBlockTagPredicate, MatchingFluidsPredicate, WouldSurvivePredicate | Предикаты блоков: true, matching_blocks, matching_block_tag, matching_fluids, all_of, any_of, not, would_survive | ЧАСТИЧНО | crates/rine-worldgen/src/feature/predicate.rs (8 типов). Используются только фичами, которые в сервере не запускаются |
| HasSturdyFacePredicate, SolidPredicate, ReplaceablePredicate, UnobstructedPredicate, HeightRangePredicate, VolumeMatchPredicate, InsideWorldBoundsPredicate, MatchingBiomesPredicate, StateTestingPredicate | Предикаты: solid, replaceable, has_sturdy_face, unobstructed, height_range, volume_match, inside_world_bounds, matching_biomes, state_testing | НЕТ | В predicate.rs их нет. Встреча такого предиката даст ошибку |
| BlockPredicate, BlockPredicateType, CombiningPredicate | Интерфейс и реестр предикатов | служебные | Служебное |
| ConstantHeight, UniformHeight, TrapezoidHeight, VeryBiasedToBottomHeight, BiasedToBottomHeight, HeightProvider, HeightProviderType | Высоты: constant, uniform, trapezoid, very_biased_to_bottom, biased_to_bottom (только как IntProvider) | ЧАСТИЧНО | crates/rine-worldgen/src/feature/placement.rs. У HeightProvider нет biased_to_bottom |
| WeightedListHeight | Взвешенный список высот (weighted_list) | НЕТ | Не реализован |

## levelgen/presets, flat, carver, blending

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| WorldPreset, WorldPresets, FlatLayerInfo, FlatLevelGeneratorPreset, FlatLevelGeneratorPresets, FlatLevelGeneratorSettings | Пресеты миров и плоский мир | НЕТ | Не реализованы. Есть только обычный overworld |
| CanyonWorldCarver, CaveWorldCarver, WorldCarver, WorldCarverTypes | Пещеры и каньоны (карверы) | ЧАСТИЧНО | crates/rine-worldgen/src/carver/ (carvers.rs, mask.rs, mod.rs). Сверены с ванилью, но живой путь вызывает Overworld::generate без карверов (generate_carved нигде не вызывается). В живом мире пещер нет |
| Blender, BlendingData | Смешивание со старыми чанками (Blender, BlendingData) | НЕТ | Не реализовано. Старых миров с другим рельефом нет |

## biome

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| Climate, MultiNoiseBiomeSource, MultiNoiseBiomeSourceParameterList, MultiNoiseBiomeSourceParameterLists, BiomeManager, BiomeResolver | Выбор биома по климату: multi-noise, параметры климата, зум BiomeManager, BiomeResolver | ЕСТЬ | crates/rine-worldgen/src/overworld/climate.rs, overworld/biome.rs. Живой путь (карта биомов в Overworld::generate) |
| Biome, Biomes, BiomeSource, BiomeSources, OverworldBiomeBuilder, BiomeGenerationSettings, FeatureSorter | Список биомов, свойства биомов, порядок фич, фичи по биомам | ЧАСТИЧНО | Id биомов берутся из реестра jar (crates/rine-data). Список для декора свой: crates/rine-world/src/gen/biome.rs (23 биома). BiomeGenerationSettings: фичи по биомам в assets/worldgen/features.json, не живые. FeatureSorter: crates/rine-worldgen/src/feature/sorter.rs, не живой |
| BiomeSpecialEffects | Эффекты биома: туман, цвета, частицы | не уверен | Не проверял, что берётся из реестра jar, а что вычисляется на сервере |
| CheckerboardColumnBiomeSource, FixedBiomeSource, TheEndBiomeSource, MobSpawnSettings | Источники биомов для Незера, Энда, плоского мира и шахматной доски; спавн мобов по биому | НЕТ | Не реализованы. MobSpawnSettings: спавна мобов нет |
| package-info (biome) | Служебный файл пакета | служебные | Служебное |

## chunk (чанки)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| LevelChunk, LevelChunkSection, ChunkAccess | Чанк, секция чанка, доступ к чанку | ЕСТЬ | crates/rine-world/src/chunk.rs (Chunk, Section). Своя реализация, живая |
| PalettedContainer, PalettedContainerRO, PalettedContainerFactory, Palette, HashMapPalette, LinearPalette, GlobalPalette, SingleValuePalette, PaletteResize, Strategy | Палитры блоков в секции: PalettedContainer и варианты палитр | ЧАСТИЧНО | Блоки хранятся в секциях своим способом: crates/rine-world/src/chunk.rs, encode.rs (сеть), storage.rs (диск). Отдельных классов палитр нет |
| DataLayer, LightChunk, LightChunkGetter | Данные света по чанку: DataLayer, LightChunk, LightChunkGetter | ЧАСТИЧНО | crates/rine-world/src/light.rs. Свой формат, свет живёт внутри чанка |
| CarvingMask, CarverOutput | Маска карверов | ЧАСТИЧНО | crates/rine-worldgen/src/carver/mask.rs. Не подключено к серверу |
| ChunkGenerator, ChunkSource, ProtoChunk | Генератор чанка, источник чанков, прото-чанк | ЧАСТИЧНО | crates/rine-world/src/gen/mod.rs (Generator), crates/rine-server/src/game/world.rs (кэш и загрузка чанков), rine-worldgen pipeline.rs (Proto). Pipeline не подключён |
| ChunkGenerators, ChunkGeneratorStructureState, StructureAccess, UpgradeData, EmptyLevelChunk, ImposterProtoChunk | Реестр генераторов, структурное состояние чанка, структуры в чанке, апгрейд старых чанков, пустой и подменный чанк | НЕТ | Не реализованы. Так как структур нет, структурного состояния тоже нет |
| Configuration | Configuration | не уверен | Не понял назначение без чтения файла. Проверить вручную |
| BlockColumn, BulkSectionAccess, MissingPaletteEntryException | Служебные классы чанков: BlockColumn, BulkSectionAccess, MissingPaletteEntryException | служебные | Служебное |

## chunk/status (стадии создания чанка)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| ChunkDependencies, ChunkPyramid, ChunkStatus, ChunkStatusTask, ChunkStatusTasks, ChunkStep, ChunkType, WorldGenContext | Стадии создания чанка (ChunkStatus, зависимости, задачи, пирамида) | ЧАСТИЧНО | Статусов нет. Похожая логика окна соседей (3x3 и 5x5) в crates/rine-worldgen/src/pipeline.rs, но он не подключён. Живой путь делает чанк одним вызовом |

## chunk/storage (хранение чанков)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| IOWorker, RegionFileStorage, SimpleRegionStorage, RecreatingSimpleRegionStorage, RegionStorageInfo, SerializableChunkData, SectionStorage | Запись и чтение чанков по регионам: IOWorker, SectionStorage, SerializableChunkData и варианты хранилища | ЧАСТИЧНО | crates/rine-world/src/storage.rs: папка на 32x32 чанка, файл на чанк c.X.Z.bin, LZ4, палитры по секциям, блок-сущности, запись через временный файл и переименование. Формат свой (RIN1), не Anvil |
| RegionFile, RegionBitmap, RegionFileVersion | Формат Anvil (.mca): RegionFile, RegionBitmap, RegionFileVersion | НЕТ | Anvil не читается и не пишется. В storage.rs сказано, что импорт Anvil запланирован отдельным инструментом |
| EntityStorage, ChunkScanAccess, ChunkIOErrorReporter | Сохранение сущностей и проверки чтения | НЕТ | Сущности не сохраняются: storage.rs пишет только блок-сущности. Отчётов об ошибках чтения чанков в ванильном виде нет |
| package-info (chunk/storage) | Служебный файл пакета | служебные | Служебное |

## lighting (освещение)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| LightEngine, LevelLightEngine, BlockLightEngine, SkyLightEngine, ChunkSkyLightSources, DynamicGraphMinFixedPoint, LeveledPriorityQueue | Расчёт света: блочный и небесный, очереди, движки | ЧАСТИЧНО | crates/rine-world/src/light.rs: расчёт внутри одного чанка. Соседи считаются тёмными, небо над чанком открыто. crates/rine-server/src/game/world.rs помечает чанки на пересчёт (relight, light_jobs). Точная семантика границ не проверена |
| BlockLightSectionStorage, LayerLightSectionStorage, SkyLightSectionStorage, DataLayerStorageMap | Хранение света по секциям и слоям | ЧАСТИЧНО | В rine-world свет лежит в чанке (light.rs), отдельного хранилища по секциям нет |
| LightEventListener, LayerLightEventListener | Интерфейсы слушателей света | служебные | Служебное |
| SpatialLongSet | SpatialLongSet | не уверен | Не понял, нужен ли аналог. Проверить вручную |

## storage (сохранение мира)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| LevelData, PrimaryLevelData, ServerLevelData, WorldData, WritableLevelData, DerivedLevelData, LevelDataAndDimensions, LevelSummary, LevelVersion, DataVersion, LevelStorageSource, LevelResource | level.dat и папка мира: данные уровня, версии, сводка мира, ресурсы | НЕТ | level.dat не пишется и не читается. Seed в конфиге, правила в gamerules.txt (crates/rine-server/src/game/cmd/rules.rs), время и погода только в памяти |
| SavedDataStorage | Сохранённые данные мира (SavedDataStorage) | ЧАСТИЧНО | Общего механизма нет. Скорборд пишется в data/scoreboard.dat (crates/rine-server/src/game/cmd/commands/scoreboard.rs, write_gzip), состояние датапаков в rine_datapacks.json (datapack.rs). Остальные сохранённые данные (карты, погода, торговец) не пишутся |
| CommandStorage | Хранилище команд (/data storage) | ЧАСТИЧНО | Команда /data с хранилищем есть: crates/rine-server/src/game/cmd/commands/data.rs. Не уверен, переживает ли оно перезапуск сервера |
| ValueInput, ValueOutput, TagValueInput, TagValueOutput, ValueInputContextHelper | Обёртки чтения и записи NBT: ValueInput, ValueOutput, TagValueInput, TagValueOutput | ЧАСТИЧНО | NBT разбирает crates/rine-protocol, команды используют crates/rine-command (snbt.rs). Отдельного слоя нет |
| PlayerDataStorage | Данные игроков | ЕСТЬ | crates/rine-server/src/game/playerdata.rs (save_player, save_all) |
| FileNameDateFormatter, LevelStorageException | Служебные классы: форматировщик имён, исключение хранилища | служебные | Служебное |
| package-info (storage) | Служебный файл пакета | служебные | Служебное |

## storage/loot (таблицы добычи)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| LootTable, LootPool, LootParams, LootContext, BuiltInLootTables, LootDataType, IntLimit | Таблицы, пулы, параметры, контекст, встроенный реестр, ограничение числа | ЧАСТИЧНО | crates/rine-server/src/game/loot.rs разбирает только block-таблицы из rine_data::raw::BLOCK_LOOT (drops блоков). Таблиц сундуков, мобов и встроенного реестра нет. Зачарований нет, поэтому фортуна и шёлковое касание не работают |
| ContainerComponentManipulator, ContainerComponentManipulators, FloatRangePredicate, IntRangePredicate | Контейнеры с компонентами, диапазоны значений в предикатах | НЕТ | Не реализованы |
| LootContextArg, LootContextUser, Validatable, ValidationContext, ValidationContextSource | Служебные классы: контексты, валидация, базовые типы | служебные | Служебное |
| package-info (storage/loot) | Служебный файл пакета | служебные | Служебное |

## storage/loot/entries (записи таблиц)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| LootItem, TagEntry, AlternativesEntry, EntryGroup, SequentialEntry, EmptyLootItem | Записи таблиц: item, tag, alternatives, group, sequence, empty | ЧАСТИЧНО | crates/rine-server/src/game/loot.rs, функция entry(). Другие типы записей не читаются |
| DynamicLoot, SlotLoot | Динамическая добыча и слотовая добыча | НЕТ | Не реализованы |
| NestedLootTable | NestedLootTable (вложенная таблица) | не уверен | В loot.rs есть упоминание loot_table. Не понял, обрабатывается ли. Проверить вручную |
| ComposableEntryContainer, CompositeEntryBase, ExpandableContainerBase, LootPoolEntries, LootPoolEntry, LootPoolEntryContainer, SingleEntryContainerBase, UniformContainerBase | Базовые классы записей | служебные | Служебное |
| package-info (storage/loot/entries) | Служебный файл пакета | служебные | Служебное |

## storage/loot/functions (функции таблиц)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| SetItemCountFunction, LimitCount, ApplyBonusCount | Функции: set_count, limit_count, apply_bonus (только как пустая операция) | ЧАСТИЧНО | crates/rine-server/src/game/loot.rs, func(). apply_bonus без зачарований не меняет количество |
| ApplyExplosionDecay | ApplyExplosionDecay (уменьшение добычи от взрыва) | не уверен | Не понял, применяется ли к взрывам. Проверить вручную, в explosion.rs добыча берётся через block_drops |
| FunctionUserBuilder, FilteredFunction, ListOperation, LootItemConditionalFunction, LootItemFunction, LootItemFunctions | Базовые классы функций | служебные | Служебное |
| CopyBlockState, CopyComponentsFunction, CopyCustomDataFunction, CopyNameFunction, DiscardItem, EnchantRandomlyFunction, EnchantWithLevelsFunction, EnchantedCountIncreaseFunction, ExplorationMapFunction, FillPlayerHead, ModifyContainerContents, SequenceFunction, SetAttributesFunction, SetBannerPatternFunction, SetBookCoverFunction, SetComponentsFunction, SetContainerContents, SetContainerLootTable, SetCustomDataFunction, SetCustomModelDataFunction, SetEnchantmentsFunction, SetFireworkExplosionFunction, SetFireworksFunction, SetInstrumentFunction, SetItemDamageFunction, SetItemFunction, SetLoreFunction, SetNameFunction, SetOminousBottleAmplifierFunction, SetPotionFunction, SetRandomDyesFunction, SetRandomPotionFunction, SetStewEffectFunction, SetWritableBookPagesFunction, SetWrittenBookPagesFunction, SmeltItemFunction, ToggleTooltips | Остальные функции: копирование состояния, имени, компонентов, зачарования, книги, зелья, фейерверки, карта, содержимое контейнеров, плавка и т.п. | НЕТ | Не реализованы. Неизвестная функция в loot.rs не делает ничего (Op::Noop), без ошибки. Это опасно: добыча молча станет другой |
| package-info (storage/loot/functions) | Служебный файл пакета | служебные | Служебное |

## storage/loot/predicates (условия таблиц)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| MatchBlock, MatchTool, LootItemRandomChanceCondition, LootItemRandomChanceWithEnchantedBonusCondition, BonusLevelTableCondition, AllOfCondition, AnyOfCondition, InvertedLootItemCondition, ExplosionCondition | Условия: match_block, match_tool, random_chance (и с бонусом), table_bonus, all_of, any_of, inverted, survives_explosion | ЧАСТИЧНО | crates/rine-server/src/game/loot.rs, cond(). Бонус удачи без зачарований даёт шанс уровня 0. survives_explosion всегда истина, радиус не учитывается |
| LootItemEntityPropertyCondition | LootItemEntityPropertyCondition (условие по свойствам сущности) | не уверен | В loot.rs есть ветка entity_properties, но не разбирал, что именно она проверяет. Проверить вручную |
| CompositeLootItemCondition, ConditionUserBuilder, LootItemCondition, LootItemConditionTypes, LootPredicates | Базовые классы условий | служебные | Служебное |
| DamageSourceCondition, EnchantmentActiveCheck, EntityHasScoreCondition, EnvironmentAttributeCheck, FloatValueCheck, IntValueCheck, LocationCheck, LootItemKilledByPlayerCondition, TimeCheck, WeatherCheck | Прочие условия: урон, зачарование, очки, атрибуты среды, числовые проверки, местоположение, время, погода, убийство игроком | НЕТ | Не реализованы. Неизвестное условие в loot.rs всегда ложно (Cond::Never), без ошибки |
| package-info (storage/loot/predicates) | Служебный файл пакета | служебные | Служебное |

## storage/loot/providers (числа в таблицах)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| UniformGenerator, BinomialDistributionGenerator, ConstantValue | Числа: uniform, binomial, constant (целые) | ЧАСТИЧНО | crates/rine-server/src/game/loot.rs, provider(). Неизвестный провайдер молча даёт 1 |
| ContextNbtProvider, NbtProvider, NbtProviders, StorageNbtProvider, AggregateProvider, BinaryProvider, ConditionalProvider, DispatcherProvider, DistributionProvider, EnvironmentAttributeProvider, PowerProvider, RangeProvider, StoredNumberAccess, UnaryProvider, Absolute, Average, Ceiling, ConditionalValue, ConstantValue, ContextFloatProvider, ContextFloatProviderTypes, ContextFloatProviders, Cosine, Difference, EnchantmentLevelProvider, EnvironmentAttributeValue, Floor, FromInt, Length, Maximum, Minimum, Modulus, Negate, NumberDispatcher, Power, Product, Quotient, ResolvableFloat, Round, Sine, SquareRoot, StorageValue, Sum, Truncate, UniformGenerator, WeightedListValue, Absolute, Average, ConditionalValue, ContextIntProvider, ContextIntProviderTypes, ContextIntProviders, Difference, EnvironmentAttributeValue, FloorModulus, FloorQuotient, FromFloat, Maximum, Minimum, Modulus, Negate, NumberDispatcher, Power, Product, Quotient, ResolvableInt, ScoreboardValue, StorageValue, Sum, WeightedListValue, ContextScoreboardNameProvider, FixedScoreboardNameProvider, ScoreboardNameProvider, ScoreboardNameProviders | Остальные провайдеры: nbt, score, дробные, вычисления и т.п. | НЕТ | Не реализованы |

## storage/loot/parameters (параметры контекста)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| LootContextParamSets, LootContextParams | Наборы параметров контекста добычи | ЧАСТИЧНО | loot.rs использует блок, инструмент и позицию. Наборы параметров отдельно не описаны |
| package-info (storage/loot/parameters) | Служебный файл пакета | служебные | Служебное |

## dimension (измерения)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| BuiltinDimensionTypes, DimensionType | Тип измерения: только overworld | ЧАСТИЧНО | crates/rine-server/src/game/mod.rs (dim_type_id из реестра, overworld). Нетера и Энда нет |
| LevelStem, DimensionDefaults, DragonRespawnStage, EnderDragonFight | Нетер, Энд, стебли измерений, бой с драконом, умолчания измерения | НЕТ | Не реализованы. /execute in the_nether и the_end разбираются (execute.rs), но отдельного мира нет |
| package-info (dimension) | Служебные файлы пакета | служебные | Служебное |

## border (граница мира)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| WorldBorder, BorderStatus | Граница мира: размер, центр, lerp, предупреждения, пакеты | ЧАСТИЧНО | crates/rine-server/src/game/cmd/commands/world.rs (Border, команда worldborder, пакеты). Эффекта на игрока нет: поиск по crates/rine-server/src/game не нашёл урона за границей и ограничения движения |
| BorderChangeListener | Слушатели границы | НЕТ | Не реализованы, слушателей нет |
| package-info (border) | Служебный файл пакета | служебные | Служебное |

## gamerules (игровые правила)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| GameRule, GameRuleMap, GameRules, GameRuleType | Правила игры: определение, хранение, чтение и запись значений | ЧАСТИЧНО | crates/rine-server/src/game/cmd/rules.rs: 58 правил, значения в gamerules.txt. В ванили 59, лишнее max_minecart_speed стоит за фича-флагом MINECART_IMPROVEMENTS и не поддержано. Код читает только 11 правил: advance_time, advance_weather, command_block_output, immediate_respawn, limited_crafting, log_admin_commands, max_block_modifications, max_command_forks, max_command_sequence_length, reduced_debug_info, send_command_feedback. Остальные 47 объявлены, но не влияют на игру |
| GameRuleCategory, GameRuleTypeVisitor | Категории правил и обход типов | НЕТ | Не реализованы |
| package-info (gamerules) | Служебный файл пакета | служебные | Служебное |

## timers (таймеры команд)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| FunctionCallback, TimerQueue | Отложенный вызов функции: /schedule function, очистка | ЕСТЬ | crates/rine-server/src/game/cmd/commands/function.rs (TimerQueue, schedule function, schedule clear) |
| TimerCallback, TimerCallbacks | Общие обратные вызовы таймеров | ЧАСТИЧНО | function.rs: только вызов функции |
| FunctionTagCallback | FunctionTagCallback (вызов тега функций) | не уверен | Не проверил, принимает ли /schedule тег функций |
| package-info (timers) | Служебный файл пакета | служебные | Служебное |

## blockscan (поиск блоков в объёме)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| BlockMatcher, BlockScanUtils, BoxBlockMatcher | Матчеры блоков и поиск блоков в объёме | ЧАСТИЧНО | crates/rine-server/src/game/cmd/commands/blocks.rs (/fill с режимами outline, hollow и др.). Отдельных матчеров и утилит по именам нет |
| FilteredSectionCache, OrderedBlockMatcher | Кэш секций и упорядоченный матчер | НЕТ | Не реализованы |
| BlockStateConsumer, package-info (blockscan) | Интерфейс потребителя состояний | служебные | Служебное |

## entity (сущности уровня)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| EntityAccess, EntityTickList, LevelEntityGetter, LevelEntityGetterAdapter, TransientEntitySectionManager | Учёт сущностей уровня и тик: отдельный список неигроковых сущностей | ЧАСТИЧНО | crates/rine-server/src/game/entity.rs: предметы, TNT, падающие блоки, параллельный тик. Общего менеджера с секциями нет |
| EntityLookup, EntitySection, EntitySectionStorage, PersistentEntitySectionManager, EntityPersistentStorage, ChunkEntities, UUIDLookup, LevelCallback, EntityInLevelCallback, ChunkStatusUpdateListener, EntityTypeTest | Секции сущностей, поиск по UUID, сохранение сущностей, колбэки и проверки типов | НЕТ | Не реализованы: секций нет, сущности не сохраняются, поиска по UUID нет. Сетка по секциям запланирована в HANDOFF (этап TNT 3) |
| Visibility | Visibility (видимость сущности) | не уверен | Не сверял с crates/rine-server/src/game/track.rs. Проверить вручную |
| UniquelyIdentifyable, package-info (entity) | Интерфейс уникального идентификатора | служебные | Служебное |

## pathfinder (поиск пути)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| AmphibiousNodeEvaluator, FlyNodeEvaluator, PathFinder, PathfindingContext, PathTypeCache, SwimNodeEvaluator, Target, WalkNodeEvaluator | Поиск пути и оценщики узлов для ходьбы, плавания, полёта, амфибий | НЕТ | Не реализовано. В Rine нет ИИ мобов и навигации |
| BinaryHeap, Node, NodeEvaluator, Path, PathComputationType, PathType, package-info (pathfinder) | Структуры данных пути | служебные | Служебное |

## saveddata (сохраняемые данные)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| MapBanner, MapDecoration, MapDecorationType, MapDecorationTypes, MapFrame, MapId, MapIndex, MapItemSavedData | Карты (filled_map): баннеры, декорации, рамки, идентификаторы, индекс, данные карты | НЕТ | Карт нет, данные карт не пишутся |
| SavedData, SavedDataType, WanderingTraderData | Общий механизм сохранённых данных и данные бродячего торговца | НЕТ | Нет механизма и данных |
| WeatherData | Данные погоды | ЧАСТИЧНО | Погода считается в памяти (crates/rine-server/src/game/cmd/commands/world.rs, advance_weather) и не сохраняется |
| package-info (saveddata) | Служебные файлы пакета | служебные | Служебное |

## portal (порталы) и validation (проверки путей)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| PortalForcer, PortalShape, TeleportTransition | Порталы Незера и Энда: создание, проверка формы, телепорт между мирами | НЕТ | Не реализованы. Правило allow_entering_nether_using_portals только объявлено |
| package-info (portal) | Служебный файл пакета | служебные | Служебное |
| ContentValidationException, DirectoryValidator, ForbiddenSymlinkInfo, PathAllowList | Проверка путей и симлинков датапаков и папок | НЕТ | Не реализовано |
| package-info (validation) | Служебный файл пакета | служебные | Служебное |

## world/level (файлы в корне)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| BaseCommandBlock | Командные блоки | ЕСТЬ | crates/rine-server/src/game/cmd/cmdblock.rs |
| ChunkPos, GameType, LightLayer, ScheduledTickAccess | Координаты чанка, тип игры, слой света, отложенные тики блоков | ЕСТЬ | ChunkPos: crates/rine-world/src/chunk.rs. GameType: crates/rine-server/src/game/cmd/commands/gamemode.rs. LightLayer: crates/rine-world/src/light.rs. ScheduledTickAccess: crates/rine-server/src/game/redstone (schedule_tick) |
| Level | Level: мир как объект | ЧАСТИЧНО | Класса уровня нет. Состояние разложено по crates/rine-server/src/game/mod.rs, world.rs и cmd/commands/world.rs: часы, погода, граница, форсированные чанки |
| Explosion, ServerExplosion, ExplosionDamageCalculator, EntityBasedExplosionDamageCalculator, SimpleExplosionDamageCalculator | Взрывы и расчёт урона от взрыва | ЧАСТИЧНО | crates/rine-server/src/game/explosion.rs: только TNT, пакетно, с упрощениями (описаны в шапке файла). Взрывов от других источников нет |
| BlockEventData, TicketStorage | События блоков и принудительная загрузка чанков | ЧАСТИЧНО | BlockEventData: события блоков в crates/rine-server/src/game/redstone/engine.rs (поршни). Смотрит другой агент по redstone, проверено поверхностно. TicketStorage: /forceload в world.rs (is_forced), без тикетов |
| BaseSpawner, CustomSpawner, LocalMobCapCalculator, NaturalSpawner, PotentialCalculator, SpawnData, Spawner | Спавн мобов: спавнеры, мобкап, естественный спавн, потенциал, данные спавна | НЕТ | Не реализовано. Блок спавнера есть в crates/rine-server/src/game/blockent.rs, но логики спавна рядом с ним не нашёл |
| StructureManager, EmptyStructureManager, PathNavigationRegion | Менеджер структур и путевой регион для навигации | НЕТ | Не реализовано |
| DataPackConfig, LevelSettings, WorldDataConfiguration | Настройки мира и датапаков | НЕТ | Не реализовано: level.dat нет |
| MoonPhase, NoiseColumn, BlockCollisions, CardinalLighting | MoonPhase, NoiseColumn, BlockCollisions, CardinalLighting | не уверен | Не проверял полностью. Фаза луны в Rine не найдена, но не уверен, что она нужна (сон и спавн зависят от неё в ванили) |
| BlockAndLightGetter, BlockGetter, ClipBlockStateContext, ClipContext, CollisionGetter, ColorMapColorUtil, ColorResolver, CommonLevelAccessor, DryFoliageColor, EmptyBlockGetter, EntityGetter, FoliageColor, GrassColor, ItemLike, LevelAccessor, LevelHeightAccessor, LevelReader, LevelSimulatedRW, LevelSimulatedReader, LevelWriter, ServerLevelAccessor, SignalGetter, WorldGenLevel | Интерфейсы доступа к миру, контексты лучей, клиентские цвета листвы и травы | служебные | Служебное. В Rine отдельных классов с такими именами нет, логика сделана функциями |
| package-info (.) | Служебный файл пакета | служебные | Служебное |

## Служебные package-info (остальные папки)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| package-info (chunk), package-info (chunk/status), package-info (dimension/end), package-info (levelgen/blending), package-info (levelgen/blockpredicates), package-info (levelgen/carver), package-info (levelgen/densityfunction/generator), package-info (levelgen/densityfunction/op), package-info (levelgen/densityfunction), package-info (levelgen/feature/featuresize), package-info (levelgen/feature/foliageplacers), package-info (levelgen/feature), package-info (levelgen/feature/rootplacers), package-info (levelgen/feature/stateproviders), package-info (levelgen/feature/treedecorators), package-info (levelgen/feature/trunkplacers), package-info (levelgen/flat), package-info (levelgen/heightproviders), package-info (levelgen/material/condition), package-info (levelgen/material), package-info (levelgen/material/rule), package-info (levelgen), package-info (levelgen/placement), package-info (levelgen/presets), package-info (levelgen/structure), package-info (levelgen/structure/pieces), package-info (levelgen/structure/placement), package-info (levelgen/structure/pools/alias), package-info (levelgen/structure/pools), package-info (levelgen/structure/structures), package-info (levelgen/structure/templatesystem/loader), package-info (levelgen/structure/templatesystem), package-info (levelgen/structure/templatesystem/rule/blockentity), package-info (levelgen/synth), package-info (lighting), package-info (saveddata/maps), package-info (storage/loot/providers/nbt), package-info (storage/loot/providers/number/floats), package-info (storage/loot/providers/number/ints), package-info (storage/loot/providers/number), package-info (storage/loot/providers/score) | Файлы описания пакетов. Механики нет | служебные | Служебное |

## Хвосты из отчёта 01 (блоки), 2026-11-16

Что блоки в `game/blocks/` ждут от мира и генерации:

- Фичи генерации в живом мире. Костная мука на траве, нилии, мхе, грибах и рост саженцев ставят фичи в мир игры, а `rine-worldgen` умеет писать только в буфер `Region`. Мост делается в отчёте 01 (`blocks/sapling.rs`); если он потребует правок `rine-worldgen`, они попадут сюда.
- Окклюзионные формы блоков. В `blocks_ext.json` их нет, поэтому трава, нилий и снег проверяют коллизию вместо окклюзии верхней грани. Для обычных блоков ответ совпадает, для блоков с разной коллизией и окклюзией может нет.
- Осадки биома для котла с водой под каплей капельника: `LayeredCauldronBlock` проверяет дождь, капля берёт биом не всегда.
- Уровень моря берётся из настроек обычного мира, а не из уровня. Нужен, когда появятся другие измерения.
- Свойство измерения `WATER_EVAPORATES` (Незер): мокрая губка и котлы ведут себя как в обычном мире.
- Взрывы: `explosion.rs` умеет только TNT пачкой. Якорю возрождения нужен взрыв с огнём и с защитой от воды рядом, кровати вне обычного мира тоже. Сейчас якорь вызывает `explode_batch` без этих флагов.
- Тики жидкости для заводнённых блоков в `updateShape` (ламинария, высушенный гаст, капельник): у движка блоков нет API запланировать тик жидкости.
