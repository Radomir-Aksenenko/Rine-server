# Аудит 01: блоки, редстоун, материалы, game events

Дата: 2026-11-15. Только чтение кода, ничего не менял.

Охват: `Paper-26.3/.../minecraft/java/net/minecraft/world/level/` в папках `block` (включая `entity`, `piston`, `state`, `sounds`, `grower`, `pattern`), `redstone`, `material`, `gameevent`. Всего 511 файлов. Ниже они собраны в группы. Тривиальные декоративные классы сведены в одну строку в конце.

Что смотрел в Rine: `crates/rine-server/src/game/` (redstone/engine.rs, redstone/piston.rs, redstone/rails.rs, redstone/data.rs, neighbors.rs, place.rs, interact.rs, blockinv.rs, containers.rs, fluids.rs, survival.rs, entity.rs, explosion.rs, mod.rs, cmd/rules.rs), `crates/rine-data/src/blocks.rs`. Остальные крейты смотрел точечно.

## Сводка

- Групп в таблице: 108. По статусу: ЕСТЬ 26, ЧАСТИЧНО 39, НЕТ 37, не уверен 6. Где не проверил, так и написано.
- Редстоун и поршни сделаны почти полностью: провод, факелы, повторитель, компаратор, наблюдатель, поршни, плиты, кнопки, рычаги, датчик света, воронка, раздатчик, TNT, лампа, медная лампа, нотный блок, колокол (сигнал), командный блок.
- Главная системная дыра: нет случайных тиков (random tick). Флаг `RANDOM_TICK` считается в `rine-data/src/blocks.rs:143`, но в игре его никто не читает. Правило `random_tick_speed` объявлено в `cmd/rules.rs:65` и не используется. Из-за этого не работают рост растений, распространение травы, опадание листьев, окисление меди, огонь, таяние льда и снега, рост кристаллов, рост тростника, кактуса, бамбука, лиан.
- Вторая дыра: нет пересчёта форм блоков. Ступени (`shape`), соединения заборов, стен, панелей и решёток не обновляются при соседних изменениях. Свойство `P::Shape` используется только для рельсов, `P::North` и соседние только для провода.
- Нет огня, порталов, спаунеров, хранилища, сколка, вибраций (`gameevent`), меню наковальни, зачарования, варки, камнереза и т. д.

## Таблица

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| redstone/Redstone, NeighborUpdater, InstantNeighborUpdater, CollectingNeighborUpdater, Orientation | Порядок обновления соседей | ЕСТЬ | `redstone/engine.rs` (`update_neighbors_at`, `java_hash_order`, `run_updates`). Порядок проверен тестом `update_neighbors_order_and_nesting` (`redstone/tests.rs`). |
| redstone/DefaultRedstoneWireEvaluator, RedstoneWireEvaluator | Сила сигнала провода | ЕСТЬ | `redstone/engine.rs` (`wire_connection_state`, `calculateTargetStrength`), тест `lever_powers_wire_line_with_decay`. |
| redstone/ExperimentalRedstoneWireEvaluator, ExperimentalRedstoneUtils | Экспериментальный провод и утилиты | не уверен | Не нашёл переключателя эксперимента. Не проверял, нужен ли он для 26.3 по умолчанию. |
| block/RedstoneWireBlock | Провод: подключения, клик | ЕСТЬ | `engine.rs` (`wire_use`, `wire_placement`). |
| block/RedstoneTorchBlock, RedstoneWallTorchBlock, BaseTorchBlock, TorchBlock, WallTorchBlock | Факел-инвертор, задержка 2 тика, сгорание на частых переключениях | ЕСТЬ | `engine.rs` (`K::Torch`, `K::WallTorch`), тесты `torch_inverts_with_two_tick_delay`, `torch_burns_out_on_fast_clock`. Свет от факелов: не уверен (не смотрел `rine-world/src/light.rs`). |
| block/RepeaterBlock, ComparatorBlock, DiodeBlock | Повторитель (задержка, блокировка), компаратор (режимы сравнения и вычитания) | ЕСТЬ | `engine.rs` (`use_block`, `placed_by`, `comparator_refresh`), тесты `repeater_delays_and_locks`, `comparator_subtract_mode`. |
| block/ObserverBlock | Наблюдатель, импульс | ЕСТЬ | `engine.rs` (`K::Observer`), тест `observer_pulses_once`. |
| block/RedstoneBlock, RedstoneLampBlock | Блок редстоуна, лампа с задержкой выключения | ЕСТЬ | `engine.rs` (`K::RedstoneBlock`, `K::Lamp`), тест `lamp_turns_off_after_four_ticks`. |
| block/DaylightDetectorBlock | Датчик дневного света, инверсия, обновление раз в 20 тиков | ЕСТЬ | `engine.rs` (`daylight_update`, `tick_block_entities`). |
| block/LeverBlock | Рычаг | ЕСТЬ | `engine.rs` (`lever_pull`), `redstone/mod.rs` (`use_block`). |
| block/ButtonBlock | Кнопки каменные (20 тиков) и деревянные (30) | ЕСТЬ | `engine.rs` (`K::Button`, `button_press`), `redstone/data.rs` (`kind_of`), тест `button_releases_after_twenty_ticks`. |
| block/BasePressurePlateBlock, PressurePlateBlock, WeightedPressurePlateBlock | Нажимные плиты, веса | ЕСТЬ | `engine.rs` (`K::Plate`, `K::Weighted`, `plate_entity_inside`), `redstone/mod.rs` (`entities_on_plates`). Точно не сверял, какие сущности нажимают деревянные плиты (в ванили любая сущность, в Rine не проверено). |
| block/TripWireBlock, TripWireHookBlock | Растяжка и крюк | ЧАСТИЧНО | `engine.rs` (`K::Tripwire`, `K::TripwireHook`, сила сигнала крюка). Срабатывание от сущностей не нашёл. Не уверен. |
| block/TargetBlock | Мишень: сигнал от попадания | ЧАСТИЧНО | `engine.rs` (`K::Target`: сброс сигнала по таймеру). Попадание снарядом не реализовано: снарядов в Rine не нашёл, не уверен. |
| block/LightningRodBlock, WeatheringLightningRodBlock | Громоотвод: сигнал после удара молнии | НЕТ | Только установка с направлением (`place.rs`). Молний нет (слова `lightning` в `game/` нет, кроме установки). |
| block/CopperBulbBlock, WeatheringCopperBulbBlock | Медная лампа: переключение по сигналу | ЕСТЬ | `engine.rs` (`copper_bulb_flip`). Варианты окисления см. строку окисления. |
| block/NoteBlock | Нотный блок: клик и сигнал, инструмент под блоком | ЕСТЬ | `engine.rs` (`K::Note`, `play_note`, `note_with_instrument`) и `use_block`. |
| block/BellBlock | Колокол: звон по сигналу и по клику | ЧАСТИЧНО | Звон по сигналу есть (`engine.rs` `ring_bell`). По клику игрока не звонит: в `use_block` нет `K::Bell`. |
| block/LecternBlock | Лекторн: сигнал от книги, GUI, положить книгу | ЧАСТИЧНО | Только выход сигнала (`K::Lectern`) и установка. Книга, GUI и клик: НЕТ. |
| block/TntBlock | TNT: поджиг огнивом и огненным зарядом, сигнал, взрыв | ЕСТЬ | `place.rs` (`interact_block`, `prime_tnt`), `engine.rs` (`K::Tnt`), `explosion.rs` (пакетные взрывы). |
| block/piston/PistonBaseBlock, PistonHeadBlock, MovingPistonBlock, PistonMovingBlockEntity, PistonMath, PistonStructureResolver, package-info | Поршни, липкие поршни, толкание, разрушение, слизь и мёд | ЕСТЬ | `redstone/piston.rs` (`is_pushable`, `resolve`, `move_blocks`, `tick_movers`, `can_stick`), тесты `piston_pushes_block_and_retracts`. |
| block/DoorBlock | Двери: переключение игроком, сигнал, звук | ЧАСТИЧНО | `engine.rs` (`K::Door`, `use_block`, `door_sound`), тест `door_halves_open_together`. Железная дверь игроком не открывается: это ванильное поведение. Управление железной дверью редстоуном не проверял (не уверен). Hinge (`P::Hinge`) не выставляется при установке. |
| block/TrapDoorBlock | Люки | ЧАСТИЧНО | `use_block` (`K::Trapdoor`), сигнал в `placement_state`. Железный люк: см. дверь (не уверен по редстоуну). Выбор стороны при установке: `place.rs` (trapdoor). |
| block/FenceGateBlock | Калитки: открыть, сигнал, направление | ЧАСТИЧНО | `use_block` (`K::FenceGate`), `placement_state`. Свойство `in_wall` (`P::InWall`) в данных есть, но не выставляется. |
| block/BaseRailBlock, RailBlock, PoweredRailBlock, DetectorRailBlock, ActivatorRailBlock, RailState | Рельсы: форма, усиление, детектор, активатор | ЧАСТИЧНО | `redstone/rails.rs` (форма, `find_signal`, `on_place`, `after_removal`). Активаторный рельс: только в списке типов (`engine.rs` 1041, 1157, 1209), логики выброса нет. Детекторный рельс выдаёт сигнал (`K::DetectorRail`), вагонеток нет. |
| block/DispenserBlock, DropperBlock | Раздатчик и выбрасыватель | ЧАСТИЧНО | `blockinv.rs` (`dispense`, `DefaultDispenseItemBehavior`), `engine.rs` (`K::Dispenser`, `K::Dropper`). Спец-поведение предметов: в `blockinv.rs` (около строки 805) сказано «items without a special behaviour are dropped». Какие предметы покрыты, не сверял полностью. |
| block/HopperBlock, entity/Hopper, HopperBlockEntity | Воронка: перенос, кулдаун | ЕСТЬ | `blockinv.rs` (`tick_hoppers`, `add_hopper`), `engine.rs` (`hopper_check`), тест `hopper_moves_one_item_every_8_ticks`. |
| block/ChestBlock, AbstractChestBlock, DoubleBlockCombiner, TrappedChestBlock, entity/ChestBlockEntity, TrappedChestBlockEntity, ChestLidController | Сундуки, двойные сундуки, ловушка, крышка | ЕСТЬ | `blockinv.rs` (`chest_update`, `chest_placement`, `lid`), `containers.rs`, `engine.rs` (`K::TrappedChest` по числу смотрящих). |
| block/EnderChestBlock, entity/EnderChestBlockEntity | Эндер-сундук | не уверен | Окно открывается (`blockinv.rs` упоминает эндер-сундуки). Личное хранилище игрока не проверял. |
| block/BarrelBlock, ShulkerBoxBlock, entity/ShulkerBoxBlockEntity | Бочка, шалкер | ЧАСТИЧНО | `blockinv.rs`, `containers.rs`: окна и содержимое есть. Анимация крышки шалкера и цветные варианты не проверял. |
| block/AbstractFurnaceBlock, FurnaceBlock, BlastFurnaceBlock, SmokerBlock, entity/AbstractFurnaceBlockEntity, FurnaceBlockEntity, BlastFurnaceBlockEntity, SmokerBlockEntity | Печи: плавка, топливо, огонь в печи | ЧАСТИЧНО | `containers.rs` (`tick`, `is_fuel`, сохранение `cooking_time_spent`/`cooking_total_time` в `blockinv.rs`). Блок-рецепты blast/smoker не проверял: не уверен. |
| block/CraftingTableBlock | Верстак 3x3 | ЕСТЬ | `containers.rs` (окно 3x3). |
| block/AnvilBlock | Наковальня: падение, урон сущностям, меню переименования | ЧАСТИЧНО | Падение: `interact.rs` (`gravity_check`, ветка `anvil`), `entity.rs` (`Kind::Falling`). Урон при падении и поломка: НЕТ. Меню: НЕТ. |
| block/EnchantingTableBlock, GrindstoneBlock, CartographyTableBlock, LoomBlock, SmithingTableBlock, StonecutterBlock, CrafterBlock | Меню: зачарование, точильный камень, картография, ткацкий, кузнечный, камнерез, крафтер | НЕТ | В `place.rs` и `containers.rs` нет окон для этих блоков. В `blockent.rs` есть только имена типов блок-сущностей у крафтера и т. п. |
| block/BrewingStandBlock | Варочная стойка | НЕТ | Только имя в `blockent.rs`. Варка и окно не реализованы. |
| block/BeaconBlock, BeaconBeamBlock, BeaconBeamOwner, entity/BeaconBlockEntity | Маяк: уровни, луч, эффекты | НЕТ | Только имя в `blockent.rs` и `redstone/data.rs`. |
| block/ConduitBlock, entity/ConduitBlockEntity | Проводник: эффекты, рамка | НЕТ | Только имя в `blockent.rs`. |
| block/CampfireBlock, entity/CampfireBlockEntity | Костёр: готовка, дым, урон | НЕТ | Только имя в `blockent.rs`. |
| block/JukeboxBlock, entity/JukeboxBlockEntity | Проигрыватель: диск, звук, компаратор | ЧАСТИЧНО | В `engine.rs` (около 1910) есть список блоков, по которому, видимо, считается сигнал компаратора. Точно не проверял. Проигрывание дисков: НЕТ. |
| block/ComposterBlock | Компостер: уровень для компаратора | ЧАСТИЧНО | `engine.rs` (`"composter" => level`). Компостирование предметов: НЕТ. |
| block/AbstractCauldronBlock, CauldronBlock, LayeredCauldronBlock, LavaCauldronBlock | Котлы: уровень, наполнение, опустошение, сигнал | ЧАСТИЧНО | Уровень для компаратора: `engine.rs` (`water_cauldron`, `lava_cauldron`, `cauldron`). Использование ведра и бутылки: НЕТ. |
| block/BeehiveBlock | Улей: мёд, пчёлы | НЕТ | Только в списке блоков с лицом к игроку (`place.rs` `faces_player`). |
| block/BedBlock, AbstractBedBlock | Кровать: установка из двух половин, сон, падение, взрыв | ЧАСТИЧНО | Установка двух половин: `place.rs` (`after_place`, `placed` для `_bed`). Падение: `survival.rs` (половина урона). Сон: НЕТ. Взрыв кровати в аду: НЕТ, не уверен (в `explosion.rs` есть слово bed, не проверял). |
| block/SignBlock, PlainSignBlock, StandingSignBlock, WallSignBlock, CeilingHangingSignBlock, WallHangingSignBlock, HangingSignBlock, entity/SignBlockEntity, SignText, SignTextSlot | Таблички: установка, текст, редактирование | ЧАСТИЧНО | Установка (стоячие, настенные, подвесные): `place.rs`. Текст и редактирование: НЕТ, в `handle.rs` нет обработчика редактирования таблички. |
| block/BannerBlock, WallBannerBlock, AbstractBannerBlock, entity/BannerBlockEntity, BannerPattern, BannerPatterns, BannerPatternLayers | Знамёна: установка, узоры | ЧАСТИЧНО | Установка и настенный вариант: `place.rs` (`wall_variant`). Узоры: НЕТ. |
| block/SkullBlock, AbstractSkullBlock, PlayerHeadBlock, PlayerWallHeadBlock, WallSkullBlock, WitherSkullBlock, WitherWallSkullBlock, PiglinWallSkullBlock, entity/SkullBlockEntity | Головы и черепа: установка, скин, эффекты | ЧАСТИЧНО | Установка: `place.rs` (`wall_variant` для `_head`, `_skull`). Скин игрока и эффект иссушения: НЕТ. |
| block/FlowerPotBlock | Горшок с растением | ЧАСТИЧНО | Опора для `potted_` в `neighbors.rs`. Посадка в горшок и клик: НЕТ. |
| block/DecoratedPotBlock, entity/DecoratedPotBlockEntity, DecoratedPotPattern, DecoratedPotPatterns, PotDecorations | Декорированный горшок | НЕТ | Только имя в `blockent.rs`. |
| block/ChiseledBookShelfBlock, entity/ChiseledBookShelfBlockEntity, ShelfBlock, ShelfMushroomBlock, entity/ShelfBlockEntity | Книжные полки, полки | НЕТ | Только имя в `blockent.rs` и список в `engine.rs` (сигнал компаратора, не уверен). |
| block/StructureBlock, entity/StructureBlockEntity, StructureVoidBlock, JigsawBlock, entity/JigsawBlockEntity, GameMasterBlock, TestBlock, TestInstanceBlock, entity/TestBlockEntity, entity/TestInstanceBlockEntity, state/properties/StructureMode, TestBlockMode | Блоки структур, джигсо, тестовые блоки | НЕТ | Только имена в `blockent.rs` и `cmd/commands/misc/place.rs`. Сохранение и загрузка структур: НЕТ. |
| block/BarrierBlock, LightBlock, package-info | Барьер и блок света | ЧАСТИЧНО | Коллизия и видимость барьера из `rine-data` (флаги). Уровень света блока `light`: не проверял (не уверен). |
| block/CommandBlock, entity/CommandBlockEntity | Командный блок: импульс, цепь, сигнал | ЕСТЬ | `cmd/cmdblock.rs`, `redstone/mod.rs` (`register_hooks("command_block")`), тест `game_command_block_impulse` в `redstone/game_tests.rs`. |
| block/SpawnerBlock, TrialSpawnerBlock, entity/SpawnerBlockEntity, TrialSpawnerBlockEntity, entity/trialspawner/* | Спаунер и триал-спаунер | НЕТ | Только имена в `blockent.rs`. Мобов в Rine нет. |
| block/VaultBlock, entity/vault/* | Хранилище триал-камеры | НЕТ | Только имена в `blockent.rs`. |
| block/CreakingHeartBlock, entity/CreakingHeartBlockEntity, state/properties/CreakingHeartState | Скрипучее сердце | НЕТ | Только имя в `blockent.rs` и worldgen-дерево. |
| block/SculkBlock, SculkBehaviour, SculkSpreader, SculkVeinBlock, SculkCatalystBlock, SculkShriekerBlock, SculkSensorBlock, CalibratedSculkSensorBlock, entity/Sculk*BlockEntity, state/properties/SculkSensorPhase | Сколк: распространение, сенсоры, визг | НЕТ | Только имена в `blockent.rs` и `redstone/data.rs`. |
| gameevent/* (GameEvent, GameEventDispatcher, GameEventListener, GameEventListenerRegistry, EuclideanGameEventListenerRegistry, DynamicGameEventListener, PositionSource, PositionSourceType, BlockPositionSource, EntityPositionSource, package-info), vibrations/* (VibrationInfo, VibrationSelector, VibrationSystem) | Игровые события и вибрации | НЕТ | Нет диспетчера событий. Отдельные реакции (сундук, компаратор, поршень) сделаны напрямую, без событий. |
| material/Fluid, FlowingFluid, WaterFluid, LavaFluid, EmptyFluid, FluidState, FluidIds, Fluids, PushReaction | Вода и лава: источники, поток, падение, уровни, лава в камень | ЕСТЬ | `fluids.rs` (`Kind`, `new_liquid`, `slope_distance`, `lava_falling_onto_water_makes_stone`), `redstone/data.rs` (`Push` для поршней). |
| material/FogType | Туман (клиентская часть) | не уверен | Серверу обычно не нужен. Не проверял. |
| material/MapColor | Цвет карты | НЕТ (не уверен) | Не нашёл использования в `game/`. Нужно только для карт. |
| block/LiquidBlock, LiquidBlockContainer, SimpleWaterloggedBlock, BucketPickup, WetSpongeBlock, SpongeBlock | Жидкие блоки, водяная обводка, ведро | ЧАСТИЧНО | Жидкости и вёдра: `fluids.rs`, `place.rs`. Водная обводка (waterlogged): `fluids.rs` (`with_waterlogged`). Губка впитывает воду: НЕТ (слова `sponge` в коде нет). |
| block/BubbleColumnBlock | Пузырьковая колонна | НЕТ | В коде нет. |
| block/FallingBlock, Fallable, ColoredFallingBlock, ConcretePowderBlock, SandBlock (через FallingBlock), BlockBehaviour падение | Песок, гравий, порошок: падение | ЧАСТИЧНО | `interact.rs` (`gravity_check`, вызывается при поломке, установке и из `cmd/commands/blocks.rs`), `entity.rs` (`Kind::Falling`, приземление в `entity::tick`). Нет: реакция на соседние изменения (взрыв, вода, поршень), урон сущностям, затвердевание порошка в воде (нигде не нашёл). |
| block/DragonEggBlock | Яйцо дракона: телепорт при клике | НЕТ | Только имя в `redstone/data.rs`. |
| block/DropExperienceBlock | Опыт при добыче руды | не уверен | Не проверял `loot.rs` на опыт. |
| block/CropBlock, BeetrootBlock, CarrotBlock, PotatoBlock, NetherWartBlock, PitcherCropBlock, TorchflowerCropBlock, StemBlock, AttachedStemBlock, CocoaBlock | Посевы: рост по стадиям, урожай | ЧАСТИЧНО | Опора (`neighbors.rs`, `wheat`, `carrots`, `potatoes`, `beetroots`, `melon_stem`, `pumpkin_stem`). Урожай при поломке: `loot.rs` (тест с семенами пшеницы). Рост: НЕТ (нет random tick). Стебли и какао: НЕТ. |
| block/BonemealableBlock, BonemealSource, BonemealableFeaturePlacerBlock | Костная мука, удобрение | НЕТ | В коде нет. |
| block/FarmlandBlock | Фермерская земля: мотыга, высыхание, затаптывание | ЧАСТИЧНО | Размещение на почве: `place.rs` (`is_soil` включает `farmland`). Мотыга превращает землю: НЕТ. Высыхание и затаптывание: НЕТ. |
| block/SaplingBlock, grower/TreeGrower, grower/package-info | Саженец: посадка и рост в дерево | ЧАСТИЧНО | Посадка: `place.rs` (`is_plant`, `_sapling`), опора `neighbors.rs`. Рост: НЕТ (random tick). |
| block/GrassBlock, SpreadingSnowyBlock, SnowyBlock, MyceliumBlock, NyliumBlock, RootedDirtBlock, PathBlock, MudBlock | Трава, мицелий, нилиум: распространение, снежность, тропинка | ЧАСТИЧНО | Флаг `snowy` следует за снегом над блоком: `neighbors.rs` (`check`). Распространение травы и мицелия (random tick): НЕТ. Тропинка от лопаты: НЕТ. |
| block/LeavesBlock, MangroveLeavesBlock, FallingParticleLeavesBlock, TintedParticleLeavesBlock, UntintedParticleLeavesBlock | Листья: опадание (distance), дроп | НЕТ опадания | Опадания нет (random tick). Дроп при добыче: `loot.rs` упоминает листья, шансы не сверял. |
| block/CactusBlock, CactusFlowerBlock, SugarCaneBlock, BambooStalkBlock, BambooSaplingBlock, KelpBlock, KelpPlantBlock, SeagrassBlock, TallSeagrassBlock | Колонны: установка на почву и воду, рост, урон кактуса | ЧАСТИЧНО | Установка кактуса и тростника: `place.rs` (`ground_ok`). Опора кактуса и тростника: `neighbors.rs`. Рост всех колонн: НЕТ. Урон кактуса: не проверял (не уверен). Бамбук: не уверен, есть ли установка. |
| block/MushroomBlock, HugeMushroomBlock, NetherFungusBlock, NetherRootsBlock, NetherSproutsBlock, HangingRootsBlock | Грибы, корни: опора, рост, огромные грибы | ЧАСТИЧНО | Опора: `neighbors.rs` (`brown_mushroom`, `crimson_fungus`, `crimson_roots`, `nether_sprouts` и др.). Рост и огромные грибы (random tick, worldgen): НЕТ. |
| block/FlowerBlock, TallFlowerBlock, DoublePlantBlock, FlowerBedBlock, TallGrassBlock, ShortDryGrassBlock, TallDryGrassBlock, DryVegetationBlock, VegetationBlock, BushBlock, EyeblossomBlock, FireflyBushBlock, LeafLitterBlock, WitherRoseBlock, SweetBerryBushBlock | Цветы, трава, кусты: опора, двойные половинки, ягоды | ЧАСТИЧНО | Опора и верхняя половина: `neighbors.rs` (`Support::UpperHalf`), `place.rs` (`TALL_PLANTS`, `half=lower`). Ягоды и урон колючек: не проверял, в `survival.rs` не нашёл. Лепестки цветочного поля: не уверен. |
| block/VineBlock, GlowLichenBlock, MultifaceBlock, MultifaceSpreader, MultifaceSpreadeableBlock, MossyCarpetBlock, HangingMossBlock, CaveVines, CaveVinesBlock, CaveVinesPlantBlock, WeepingVines, WeepingVinesPlantBlock, TwistingVines, TwistingVinesPlantBlock, NetherVines, GrowingPlantBlock, GrowingPlantBodyBlock, GrowingPlantHeadBlock, BigDripleafBlock, BigDripleafStemBlock, SmallDripleafBlock, SporeBlossomBlock | Подвесные и ползущие растения: опора, рост, распространение | ЧАСТИЧНО | Лазанье по лиане: `survival.rs` (`climbable`). Опора для ряда (`small_dripleaf` и др.): `neighbors.rs`. Рост, распространение (random tick), большие листья: НЕТ. |
| block/ChorusFlowerBlock, ChorusPlantBlock | Хорус: рост, распад | НЕТ | В коде нет. |
| block/TurtleEggBlock, SnifferEggBlock, FrogspawnBlock, SeaPickleBlock, CoralBlock, CoralFanBlock, CoralWallFanBlock, CoralPlantBlock, BaseCoralFanBlock, BaseCoralWallFanBlock, BaseCoralPlantBlock, BaseCoralPlantTypeBlock, CoralPlantTypeBlock | Яйца, кораллы: поведение (высыхание, рост, вылупление) | НЕТ | Опора для `frogspawn` в `neighbors.rs`. Поведение яиц и кораллов: НЕТ. |
| block/SoulFireBlock, BaseFireBlock, FireBlock | Огонь: распространение, урон, сгорание | НЕТ | `flint_and_steel` используется только для поджига TNT (`place.rs`). Огонь как блок не реализован. |
| block/NetherPortalBlock, Portal, EndPortalBlock, EndPortalFrameBlock, EndGatewayBlock, entity/TheEndPortalBlockEntity, entity/TheEndGatewayBlockEntity | Порталы в Незер и Энд, шлюз | НЕТ | Только имена в `blockent.rs` и `redstone/data.rs`. |
| block/RespawnAnchorBlock | Якорь возрождения | НЕТ | Только имя в `redstone/data.rs`. |
| block/WebBlock | Паутина: замедление | НЕТ | Только имя в `redstone/data.rs`. |
| block/HoneyBlock, block/SlimeBlock | Мёд: скольжение, сброс падения. Слизь: отскок, липкость | ЧАСТИЧНО | Мёд: `survival.rs` (`honey_block`, сброс падения и звук). Липкость для поршней: `redstone/piston.rs` (`sticky_block`, `can_stick`). Отскок слизи: не нашёл (не уверен). |
| block/ScaffoldingBlock | Леса: лазанье, нестабильность | ЧАСТИЧНО | Лазанье: `survival.rs` (`climbable`). Нестабильность и падение лесов: НЕТ. |
| block/HayBlock | Тюк сена: падение, уменьшение урона | не уверен | В `survival.rs` есть слово `hay`, не проверял. |
| block/PowderSnowBlock, SnowLayerBlock | Порошковый снег: проваливание, звук падения. Слой снега: опора, накопление, таяние | ЧАСТИЧНО | `survival.rs` (`powder_snow`, только звук), `neighbors.rs` (опора снега, `floor_blocked` для льда). Проваливание сущностей, накопление и таяние: НЕТ. Правило `max_snow_accumulation_height` есть в `cmd/rules.rs`, его использование не проверял. |
| block/IceBlock, FrostedIceBlock, entity/* (лёд) | Лёд: таяние, замерзание, превращение | НЕТ | Только блоки. Таяние и замерзание (random tick): НЕТ. |
| block/AmethystBlock, AmethystClusterBlock, BuddingAmethystBlock | Аметист: кристаллы, рост, растущий аметист | ЧАСТИЧНО | Установка кристаллов: `place.rs` (`amethyst_cluster`, `amethyst_bud`, направление). Рост и распространение: НЕТ (random tick). |
| block/CopperChestBlock, WeatheringCopperChestBlock, CopperGolemStatueBlock, WeatheringCopperGolemStatueBlock, entity/CopperGolemStatueBlockEntity, WeatheringCopperBarsBlock, WeatheringCopperBulbBlock, WeatheringCopperChainBlock, WeatheringCopperDoorBlock, WeatheringCopperFullBlock, WeatheringCopperGrateBlock, WeatheringCopperSlabBlock, WeatheringCopperStairBlock, WeatheringCopperTrapDoorBlock, WeatheringLanternBlock, WeatheringCopper, WeatheringCopperCollection, ChangeOverTimeBlock | Медь: окисление по времени (ChangeOverTimeBlock) | НЕТ (окисления) | Окисление требует random tick: НЕТ. Варианты блоков стоят как обычные блоки из `rine-data`. Статуэтка голема: НЕТ. |
| block/CandleBlock, AbstractCandleBlock, CandleCakeBlock, CakeBlock | Свечи, торт: установка, зажигание, поедание | НЕТ | Только имена в `redstone/data.rs`. |
| block/StairBlock, state/properties/StairsShape | Ступени: направление и половина | ЧАСТИЧНО | `place.rs` (`_stairs`: `facing`, `half`). Форма (внутренний и внешний угол, `shape`) не пересчитывается: НЕТ. |
| block/FenceBlock, WallBlock, CrossCollisionBlock, IronBarsBlock, StainedGlassPaneBlock, state/properties/WallSide | Заборы, стены, панели, решётки: соединения | НЕТ соединений | В `place.rs` нет логики соединений. Коллизия забора (1.5 блока) учтена в `entity.rs`. Соседние обновления соединений: НЕТ. |
| block/SlabBlock | Полублоки: верх, низ, двойной | ЕСТЬ | `place.rs` (`_slab`, `type`), `interact.rs` (слияние в двойной). |
| block/CarpetBlock, WoolCarpetBlock | Ковры: опора | ЕСТЬ | `neighbors.rs` (`_carpet`). |
| block/LadderBlock | Лестница: опора на стене, лазанье | ЕСТЬ | `neighbors.rs` (`ladder` как `Support::Side`), `survival.rs` (`climbable`). |
| block/LanternBlock, WeatheringLanternBlock, ChainBlock, RodBlock, EndRodBlock, LightningRodBlock | Фонари, цепь, стержни: установка, подвес | ЕСТЬ | `place.rs` (`end_rod`, `lightning_rod`, `axis`), `neighbors.rs` (фонарь, `hanging`). Сигнал громоотвода: см. выше. |
| block/SupportType, block/SegmentableBlock, block/SelectableSlotContainer, block/ColorCollection, block/WeatheringCopperCollection | Вспомогательные интерфейсы и типы | не уверен | Не отдельная механика, выполняются через конкретные блоки. |
| block/BlockBehaviour/state/* (Block, BlockState, StateDefinition, StateHolder, properties/*, pattern/*, predicate/*) | Состояния блоков, свойства, паттерны | ЕСТЬ (состояния) | Состояния и свойства: `rine-data/src/blocks.rs`, `redstone/data.rs` (`P`, `PROP_NAMES`). Паттерны (`BlockPattern`) для големов и визера: НЕТ, мобов нет. |
| block/entity/BlockEntity, BlockEntityType, BlockEntityTypes, BlockEntityTypeIds, BlockEntityTicker, TickingBlockEntity, BaseContainerBlockEntity, RandomizableContainerBlockEntity, ContainerOpenersCounter, ListBackedContainer, LidBlockEntity, BoundingBoxRenderable, BeaconBeamOwner, package-info | Базовые классы блок-сущностей | ЧАСТИЧНО | `blockent.rs` (тип по имени блока, NBT, сохранение). Тикинг и содержимое: `containers.rs`, `blockinv.rs`. Общего реестра тикеров нет. |
| block/entity/PotentSulfurBlockEntity, block/PotentSulfurBlock, block/SulfurSpikeBlock, state/properties/PotentSulfurState | Серный блок и шипы | НЕТ | Только имена в `blockent.rs`. |
| block/SpeleothemBlock, PointedDripstoneBlock, state/properties/SpeleothemThickness, Tilt | Капельный камень: падение, рост, сигнал | НЕТ | Генерация есть в `rine-worldgen`. Поведение капели: НЕТ. |
| block/DriedGhastBlock | Высушенный гаст | НЕТ | В коде нет. |
| block/HeavyCoreBlock | Тяжёлое ядро | НЕТ | В коде нет. |
| block/SuspiciousEffectHolder, BrushableBlock, entity/BrushableBlockEntity | Подозрительный песок и щётка | НЕТ | Только имена в `blockent.rs` и `redstone/data.rs`. |
| Декоративные блоки без механики: стеклянные и цветные блоки, ступени/стены/плиты из дерева и камня, остальные варианты дерева и камня (AbstractBannerBlock и т. п. уже выше) | Декоративные блоки без механики | ЕСТЬ (как состояния) | Ставятся как обычные блоки из `rine-data` (`blocks.rs`). Отдельной механики нет. Прозрачность и коллизия: флаги в `rine-data`. |
| block/Block, Blocks, RenderShape, Rotation, Mirror, Portal, LevelEvent | Реестр, рендер, поворот структур, события уровня | ЧАСТИЧНО | Реестр и флаги: `rine-data`. Поворот и зеркало (структуры): НЕТ. События уровня: используются отдельные (`2001` разрушение, `1000` выброс, `2000` дым). |
| block/sounds/AmbientDesertBlockSoundsPlayer, AmbientLeavesBlockSoundPlayer, block/SoundType | Фоновые звуки блоков, таблица звуков | ЧАСТИЧНО | Звуки по событиям: `track.rs` (`sound_near`), `interact.rs`. Фоновые плееры: НЕТ. Таблица `SoundType`: не сверял целиком. |
| block/RedStoneOreBlock | Редстоуновая руда: свечение при касании | НЕТ | Только имя в `redstone/data.rs`. Косметика, без сигнала. |
| block/InfestedBlock, InfestedRotatedPillarBlock | Блоки-паразиты | НЕТ | Только имена в `redstone/data.rs`. |

## Самое важное, чего нет

1. **Случайные тики (random tick).** Блокирует рост растений и саженцев, распространение травы, мицелия и нилиума, опадание листьев, окисление меди, огонь, таяние льда и снега, рост аметиста, колонн и лиан, высыхание фермерской земли. Код: `rine-data/src/blocks.rs:143` считает флаг, в игре он не читается; `cmd/rules.rs:65` объявляет `random_tick_speed` без использования.
2. **Огонь и распространение огня.** Не сделаны огонь как блок, его урон, сгорание и распространение. Поджиг только у TNT.
3. **Порталы** в Незер и Энд, шлюз, портал-рамка: нет.
4. **Костная мука и рост растений.** Нет удобрения и стадий роста (см. пункт 1).
5. **Соединения и формы блоков.** Заборы, стены, панели, решётки, ступени: при соседних изменениях форма не пересчитывается. Дверь не получает `hinge` при установке.
6. **Меню блоков.** Наковальня (ввод имени), зачарование, варка, точильный камень, картография, ткацкий, кузнечный, камнерез, крафтер: окон нет. Из меню реализованы верстак, сундуки, печи.
7. **Таблички.** Нельзя написать текст и отредактировать табличку, нет обработчика редактирования.
8. **Кровать.** Сон, установка времени и взрыв в аду: нет.
9. **Компараторные блоки без логики.** Колокол по клику, лекторн, котлы (наполнение), компостер (компост), торт, свечи: нет.
10. **Игровые события и вибрации** (`gameevent`), сколк, спаунеры, триал-спаунер, хранилище, якорь возрождения, маяк, проводник, варочная стойка, костёр: нет.
11. **Падающие блоки.** Песок и гравий падают только при установке, поломке и вызове из команд. Нет реакции на взрыв, воду и поршень. Наковальня не наносит урон.
12. **Медь.** Окисление по времени не реализовано, поэтому варианты меди стоят как статичные блоки.
13. **Громоотвод, молнии, активаторный рельс.** Сигнала и выброса нет.

## Не уверен

- Экспериментальный провод (`ExperimentalRedstoneWireEvaluator`): не проверял, нужен ли он для 26.3.
- Срабатывание растяжки и мишени от сущностей и снарядов, потому что не искал снаряды.
- Управление железной дверью и железным люком редстоуном (игроком они не открываются, это ванильное поведение).
- Какие предметы поддерживают спец-поведение раздатчика.
- Блок-рецепты печи для blast и smoker.
- Личное хранилище эндер-сундука.
- Дроп опыта при добыче руд, шансы дропа листьев, урон кактуса, отскок слизи, поведение сена.
- Свет от блоков (`light`), `MapColor`, `FogType`, `SoundType` целиком.
- Использование `max_snow_accumulation_height` и нахождение джукбокса в списке сигнала компаратора.
- Строки с «ЕСТЬ» для деревянных плит (какие сущности нажимают) проверены не до конца.
