# Аудит 03: сущности (ванильный `world/entity` против Rine)

Дата: 2026-11-15. Ванила: `Paper-26.3/paper-server/src/minecraft/java/net/minecraft/world/entity` (727 файлов). Rine: `crates/rine-server/src/game/` (в основном `entity.rs`, `summon.rs`, `interact.rs`, `blockinv.rs`, `explosion.rs`), `crates/rine-data`.

## Сводка

- В ванили 161 тип сущности (`EntityTypes.java`). Rine создаёт и ведёт только три неигровых: предмет (`item`), TNT (`tnt`), падающий блок (`falling_block`), и сущность игрока.
- По типам: ЕСТЬ 0, ЧАСТИЧНО 4 (предмет, TNT, падающий блок, игрок), НЕТ 157.
- Второго хранилища сущностей нет. В `entity.rs` перечисление `Kind` содержит только `Item`, `Tnt`, `Falling`. Вызовы `out::add_entity` в Rine это эти три вида (`entity.rs`) и игроки (`track.rs`). `/summon` (`summon.rs`) принимает только `item`, `tnt`, `falling_block`, остальное даёт `commands.summon.failed`.
- ИИ нет совсем: ни целей, ни мозга, ни сенсоров, ни навигации (`ai/`, 277 файлов: НЕТ, кроме атрибутов игрока).
- Здоровья и урона у мобов нет. `interact.rs` (`hurt`, `attack`) и `/damage` работают только с игроками.
- Снарядов нет: стрелы, снежки, жемчуг, огненные шары, заряды ветра. Раздатчик бросает их как обычные предметы (`blockinv.rs`, `drop_one`).
- Транспорта нет: `/ride` возвращает ошибку (`cmd/commands/entities.rs`, `mount`). Лодки и вагонетки не создаются.
- Нет декораций (рамки, картины, стойки для брони), жителей и торговцев, боссов (дракон, иссушитель), сфер опыта, молнии и сущностей отображения.
- Частично: предмет на земле (движение, слияние, таймер 6000 тиков, подбор игроком и воронкой; нет воды, лавы и огня), падающий блок (падение, установка и выпадение предмета при занятом месте; нет урона сущностям), TNT (запал, физика и взрыв есть; нет `block_state` и `explosion_power` из NBT), игрок (здоровье, урон, опыт, эффекты, атрибуты; модификаторы эффектов не применяются).
- Флаги `NoGravity`, `Silent`, `Glowing`, `Invulnerable` сохраняются в NBT (`summon.rs`, `data.rs`), но в физике не читаются (по grep в `crates/rine-server/src`).
- Реестры вариантов (волк, кошка, лягушка, свинья, корова, курица, картина, зомби-наутилус) уходят клиенту в configuration, но выбора вариантов и спавна нет. Это данные, а не механика.
- Естественного спавна мобов нет: по коду не найден ни один путь, кроме `/summon`.
- Не проверено: TNT и предметы в воде и огне подробно, полный набор флагов `Abilities`, `StackedContents`, все коды `EntityEvent`. Такие места помечены «не уверен» или «не сверял».

Покрытие: все 727 файлов входят в какую-то строку. Вспомогательные классы папок `monster/` и `animal/` (ИИ, варианты, сборщики) не перечислены по одному: они закрыты строкой на сам моб. По папкам: корень 85, ai 277, animal 130, monster 84, projectile 37, vehicle 24, boss 24, npc 15, player 14, decoration 13, item 4, variant 11, ambient 3, raid 4, schedule 2.

## Таблица по группам

### Корневые файлы `world/entity`

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| Entity.java | Базовый класс сущности: позиция, скорость, uuid, возраст, теги | ЧАСТИЧНО | `game/entity.rs` (поля `Ent`: eid, uuid, pos, vel, age), теги и флаги в `EntMeta` (`cmd/mod.rs`). Флаги `NoGravity`, `Silent`, `Glowing`, `Invulnerable` сохраняются в NBT (`summon.rs`, `data.rs`), но в физике не читаются (по grep). Огонь, воздух, вода и общий `tick` не портированы. |
| EntityType.java, EntityTypes.java, EntityTypeIds.java | Реестр типов сущностей (161 тип) | ЧАСТИЧНО | Id и имена из `rine-data/assets/entities.json` (`rine-data/src/items.rs`, `EntityInfo`). Создать можно только 3 типа: `item`, `tnt`, `falling_block` (`summon.rs`). Остальные типы в таблице ниже. |
| EntityDimensions.java | Размеры хитбокса по типу | ЧАСТИЧНО | Размеры захардкожены в `entity.rs` (`Kind::size`): предмет 0.25, остальное 0.98. Таблицы размеров нет. |
| EntityEvent.java | Коды событий сущности (статус оператора, поломка, еда, смерть) | ЧАСТИЧНО | Есть отдельные коды через `out::entity_event` (`out.rs`): статус оператора 24+уровень (`commands.rs`, `mod.rs`), поломка предмета (`durability.rs`), конец еды (`survival.rs`), код 3 (`interact.rs`, вероятно смерть, не сверял). Класс `EntityEvent` целиком не портирован. |
| EntityAttachment.java, EntityAttachments.java | Точки крепления (пассажиры, поводок) | НЕТ | Нет. Езды и поводков нет. |
| EntityFluidInteraction.java, InsideBlockEffectApplier.java, InsideBlockEffectType.java | Эффекты внутри блоков (паутина, порошковый снег, вода) для сущностей | НЕТ | В `entity.rs` такой обработки нет. Игрок в воде и порошке обрабатывается отдельно в `survival.rs`. |
| EntityProcessor.java, PostSpawnProcessor.java, PortalProcessor.java | Обработка после спавна и порталы | НЕТ | Нет. Порталов Незера нет. |
| EntityReference.java, Relative.java | Ссылки на сущности и флаги относительного телепорта | ЧАСТИЧНО | Ссылки в Rine идут через eid и `by_eid`. Телепорт с относительными координатами есть в `teleport.rs`, полностью с `Relative` не сверял. |
| EntitySelector.java | Предикаты выбора сущностей | ЧАСТИЧНО | Класс ванили не портирован. Селектор команд реализован отдельно в `cmd/select.rs`, это другой код, в зачёт не засчитываю. |
| EntitySpawnReason.java, EntitySpawnRequest.java, SpawnGroupData.java, SpawnPlacementType.java, SpawnPlacementTypes.java, SpawnPlacements.java | Причины спавна и правила размещения мобов | НЕТ | Естественного спавна нет. Спавн только через `/summon` для трёх типов. |
| ConversionType.java, ConversionTracker.java, ConversionParams.java | Превращения (зомби-житель, утопленник) | НЕТ | Нет. |
| DropChances.java, EntityEquipment.java, EquipmentTable.java, EquipmentUser.java, EquipmentSlotGroup.java, EquipmentSlot.java, SlotAccess.java, SlotProvider.java | Экипировка и шансы выпадения | ЧАСТИЧНО | Слоты игрока есть: броня в NBT-слотах 100-103, слот 150 (`data.rs`), предмет в руке (`held` в `player.rs`). У мобов экипировки и шансов выпадения нет. |
| LivingEntity.java, Avatar.java | Живые сущности: здоровье, урон, смерть, движение | ЧАСТИЧНО | Только игрок: `Player.health` (`player.rs`), `interact.rs` (`hurt`, `attack`, `knock`), `survival.rs`. Других живых сущностей нет. |
| Mob.java, PathfinderMob.java, AgeableMob.java, TamableAnimal.java, Targeting.java, Attackable.java, NeutralMob.java, OwnableEntity.java, Leashable.java, Shearable.java, Bucketable.java, ItemSteerable.java, ItemBasedSteering.java, PlayerRideable.java, PlayerRideableJumping.java, HasCustomInventoryScreen.java, ContainerUser.java, ItemOwner.java, TraceableEntity.java, ReputationEventHandler.java | Базовые классы и интерфейсы мобов: приручение, езда, цели атаки, стрижка, ведро | НЕТ | Нет. Мобов нет. `interact.rs` атакует только игроков (`attack`, поиск по `by_eid`). |
| MobCategory.java | Категории мобов для лимитов спавна | НЕТ | Нет. Лимитов спавна нет, категорий в Rine нет. |
| package-info.java | Описание пакета | НЕТ | Служебный файл, не механика. |
| ExperienceOrb.java | Сфера опыта | НЕТ | Нет. См. таблицу по типам, группа 2. |
| LightningBolt.java | Молния | НЕТ | Нет. |
| Marker.java, Interaction.java, AreaEffectCloud.java, OminousItemSpawner.java | Служебные сущности | НЕТ | Нет. |
| Display.java | Сущности отображения (блок, предмет, текст) | НЕТ | Нет. |
| Pose.java | Позы (присед, плавание, полёт на элитрах) | ЧАСТИЧНО | Присед и бег приходят от клиента (`README.md`). Своего enum поз нет. Плавание и элитры не проверял. |
| HumanoidArm.java | Основная рука | ЧАСТИЧНО | Выбранный слот руки есть (`held` в `player.rs`). Свойство «левша» не проверял. |
| MoverType.java, MoveSimulationType.java | Тип движения (сам, игрок, поршень, шалкер) | ЧАСТИЧНО | Движение с коллизией есть (`entity.rs`, `move_ent`) для предметов, TNT и падающих блоков. Типов движения (поршень, шалкер) нет. |
| InterpolationHandler.java, InterpolationTracker.java, AbstractInterpolationHandler.java, LinearInterpolationHandler.java, SteppedInterpolationHandler.java, SteppedInterpolationTracker.java, PositionMoveRotation.java, PositionPath.java, PositionStep.java, UpdateInterval.java | Сглаживание движения на клиенте и интервалы обновлений | НЕТ | Отдельного слоя интерполяции нет. Сервер шлёт позиции из `entity.rs` (`moves`, `sent_pos`) по порогу. Это другой механизм, не портирование. |
| AnimationState.java, WalkAnimationState.java, ElytraAnimationState.java | Клиентские анимации | НЕТ | Серверных анимаций сущностей нет (`swing` в `entities.rs` только для игроков). |
| Crackiness.java | Степень повреждения (трещины) | не уверен | Не уверен, для какой сущности класс: не проверял. |
| SulfurCubeArchetype.java, SulfurCubeArchetypes.java | Архетипы серного куба | НЕТ | Нет. Реестр `sulfur_cube_archetype` уходит клиенту (`registry.rs`), сущности нет. |

### Игрок (`player/`, 14 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| player/Player.java | Игрок: здоровье, инвентарь, режим, позиция | ЧАСТИЧНО | `game/player.rs`, `game/handle.rs`, `game/track.rs` (спавн игрока), `game/survival.rs`. Многое есть, но не всё из `Player.java`. |
| player/Inventory.java, player/PlayerEquipment.java, player/StackedContents.java, player/StackedItemContents.java | Инвентарь, экипировка, учёт предметов для рецептов | ЧАСТИЧНО | Инвентарь и слоты есть (`actions.rs`, `data.rs`, `containers.rs`). Учёт предметов для рецептов (`StackedContents`) не проверял, не уверен. |
| player/Abilities.java | Способности (полёт, неуязвимость, скорость) | ЧАСТИЧНО | Пакет способностей отправляется (`out.rs`, `C_PLAYER_ABILITIES`) и принимается (`handle.rs`, `S_PLAYER_ABILITIES`). Есть `gamemode` (`player.rs`). Полный набор флагов и логику полёта не сверял построчно. |
| player/Input.java | Ввод игрока с клиента (вперёд, назад, прыжок, присед) | ЧАСТИЧНО | Ввод приходит по сети (`net.rs`, `handle.rs`). Присед и бег есть (`README.md`), остальные флаги не проверял. |
| player/ChatVisiblity.java | Видимость чата | не уверен | Чат есть. Настройка видимости не проверялась. |
| player/PlayerSkin.java, player/PlayerModelPart.java, player/PlayerModelType.java | Скин и видимые части модели | не уверен | Не проверял, отправляет ли Rine части модели игрока. |
| player/ProfileKeyPair.java, player/ProfilePublicKey.java | Ключи профиля для подписи чата | НЕТ | Не найдено по имени. Подписей чата в Rine нет (содержимое не сверял). |
| player/package-info.java | Описание пакета | НЕТ | Служебный файл, не механика. |

### Предметы, TNT, падающие блоки (`item/`, 4 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| item/ItemEntity.java | Предмет на земле | ЧАСТИЧНО | См. строку «Предмет» в таблице по типам (`entity.rs`). |
| item/FallingBlockEntity.java | Падающий блок | ЧАСТИЧНО | См. строку «Падающий блок» в таблице по типам (`entity.rs`). |
| item/PrimedTnt.java | Поджигаемый TNT | ЧАСТИЧНО | См. строку «TNT» в таблице по типам (`entity.rs`, `explosion.rs`). Нет полей `block_state`, `explosion_power`. |

### Ai (`ai/`, 277 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| ai/goal/ (73) | Цели ИИ: атака, побег, питание, блуждание, следование | НЕТ | Мобов нет. Ни одна цель не портирована. |
| ai/behavior/ (119) | Поведения мозга (BehaviorControl) | НЕТ | Нет. Слово `brain` в коде Rine встречается только в именах блоков (кораллы). |
| ai/sensing/ (27) | Сенсоры: видимость, игроки рядом, цели | НЕТ | Нет. |
| ai/navigation/ (7) | Поиск пути (PathNavigation и узлы) | НЕТ | Нет. Слова `pathfind` в коде нет. |
| ai/control/ (9) | Контроллеры движения мобов (взгляд, ход, прыжок) | НЕТ | Нет. Движение игрока приходит от клиента, движения мобов нет. |
| ai/attributes/ (9) | Атрибуты сущностей (скорость, урон, здоровье) | ЧАСТИЧНО | Только у игрока: `effect.rs` (`/attribute`, эффекты). Модификаторы эффектов (скорость, прибавка здоровья) не применяются, это записано в комментарии `effect.rs`. |
| ai/memory/ (8) | Память мозга | НЕТ | Нет. |
| ai/targeting/ (2) | Выбор цели | НЕТ | Нет. |
| ai/util/ (8) | Утилиты ИИ (случайные точки, пути) | НЕТ | Нет. |
| ai/village/ (9) | Деревня, точки работы жителей | НЕТ | Нет. Деревень нет. |
| ai/gossip/ (3) | Сплетни и репутация жителей | НЕТ | Нет. |
| ai/ (корень: Brain, ActivityData, package-info) | Мозг сущности и его активности | НЕТ | Нет. `Brain` не портирован. |

### Варианты, амбиент, рейды, расписание

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| variant/ (11 файлов) | Выбор вариантов сущностей по биому, луне, структурам | ЧАСТИЧНО | Только реестры: имена `wolf_variant`, `cat_variant`, `frog_variant`, `pig_variant`, `cow_variant`, `chicken_variant`, `painting_variant` уходят клиенту в configuration (`rine-data/src/registry.rs`). Логики выбора (`SpawnConditions`, `BiomeCheck`, `MoonBrightnessCheck`, `StructureCheck`) нет. |
| ambient/ (AmbientCreature, Bat) | Летучие мыши и окружающие мирные | НЕТ | Нет. Летучей мыши нет. |
| raid/ (Raid, Raider, Raids, package-info) | Рейды (набеги разбойников) | НЕТ | Нет. |
| schedule/ (Activity, package-info) | Расписание активностей жителей | НЕТ | Нет. |

### Декорации (`decoration/`, 13 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| decoration/ArmorStand.java, Cushion.java, GlowItemFrame.java, ItemFrame.java, HangingEntity.java, BlockAttachedEntity.java, LeashFenceKnotEntity.java, Mannequin.java | Декоративные сущности: стойки, рамки, узлы поводка, манекены | НЕТ | Нет. Декораций в Rine нет, `/summon` отказывает. |
| decoration/painting/Painting.java, PaintingVariant.java, PaintingVariants.java | Картины и их варианты | ЧАСТИЧНО | Только реестр: `painting_variant` уходит клиенту. Самих картин нет. |

### Снаряды (`projectile/`, 37 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| projectile/Projectile.java, ThrowableProjectile.java, ProjectileUtil.java, ProjectileDeflection.java, ItemSupplier.java | Базовые классы снарядов, попадания, отражение | НЕТ | Нет. Снарядов нет. |
| projectile/arrow/ (5 файлов: AbstractArrow, Arrow, SpectralArrow, ThrownTrident, package-info) | Стрелы и трезубцы | НЕТ | Нет. Луков и трезубцев как снарядов нет. В `durability.rs` учтён износ трезубца, стрельбы нет. |
| projectile/hurtingprojectile/ (11 файлов: огненные шары, череп иссушителя, заряды ветра) | Огненные шары, черепа, заряды ветра | НЕТ | Нет. |
| projectile/throwableitemprojectile/ (9 файлов: снежки, яйца, жемчуг, зелья, склянки опыта) | Бросаемые предметы | НЕТ | Нет. Жемчуг Эндера есть только как правило `ender_pearls_vanish_on_death` (`cmd/rules.rs`), броска нет. |
| projectile/EvokerFangs.java, EyeOfEnder.java, FireworkRocketEntity.java, FishingHook.java, LlamaSpit.java, ShulkerBullet.java | Клыки, око Края, фейерверки, поплавок, плевок ламы, снаряд шалкера | НЕТ | Нет. Таблица лута рыбалки в `itemcmd.rs` не загружается. |

### Транспорт (`vehicle/`, 24 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| vehicle/VehicleEntity.java, DismountHelper.java, ContainerEntity.java | Базовый класс транспорта, спешивание, контейнеры транспорта | НЕТ | Нет. `/ride` отказывает (`cmd/commands/entities.rs`). |
| vehicle/boat/ (7 файлов: лодки, сундучные лодки, плоты) | Лодки и плоты | НЕТ | Нет. Лодка есть только как топливо печи (`containers.rs`). |
| vehicle/minecart/ (13 файлов: вагонетки разных видов, поведение на рельсах) | Вагонетки | НЕТ | Нет. Рельсы в `redstone/rails.rs` относятся к редстоуну, вагонеток не моделируют. |

### Боссы (`boss/`, 24 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| boss/enderdragon/ (22 файла: дракон, кристалл, фазы полёта и атаки) | Дракон Края и его фазы | НЕТ | Нет. Края не реализовано. |
| boss/wither/ (2 файла: иссушитель) | Иссушитель | НЕТ | Нет. |

### NPC, жители, торговцы (`npc/`, 15 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| npc/Npc.java, npc/ClientSideMerchant.java, npc/InventoryCarrier.java, npc/CatSpawner.java | Общий NPC, клиентский торговец, спавнер кошек | НЕТ | Нет. |
| npc/villager/ (7 файлов: житель, профессии, типы, данные) | Житель, профессии, торговля | НЕТ | Нет. Жителей и торговли нет. |
| npc/wanderingtrader/ (3 файла) | Бродячий торговец и его спавнер | НЕТ | Нет. |

### Все типы сущностей по одной строке (`EntityTypes.java`, 161 тип)

Одна строка на тип из `EntityTypes.java`. Файл класса взят из `world/entity` (`animal/`, `monster/`, `projectile/`, и т.д.). Если класс найден в нескольких папках, перечислены все пути.

#### 1. Игрок (1)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `player/Player.java` | Игрок как сущность: здоровье, урон, опыт, эффекты | ЧАСТИЧНО | `game/player.rs`, `game/interact.rs` (урон, откидывание, атака только по игрокам), `game/cmd/commands/experience.rs` (опыт), `game/cmd/commands/effect.rs` (эффекты и атрибуты, модификаторы эффектов не применяются), `game/survival.rs` (голод, утопание, void, лава). Полный `Abilities` (полёт) не проверял. |

#### 2. Неживые и служебные сущности (13)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `AreaEffectCloud.java` | Облако эффекта | НЕТ | Нет. Такой сущности в Rine нет, `/summon` отказывает. |
| `Display.java` | Блок-дисплей | НЕТ | Нет. Сущностей отображения нет. |
| `boss/enderdragon/EndCrystal.java` | Кристалл Края | НЕТ | Нет. Края не реализовано. |
| `ExperienceOrb.java` | Сфера опыта: падает с мобов, руды и печи, игрок подбирает | НЕТ | Сфер нет. Опыт игрока хранится в `game/cmd/commands/experience.rs`, сущности опыта нет. |
| `item/FallingBlockEntity.java` | Падающий песок и гравий | ЧАСТИЧНО | `game/entity.rs` (`Kind::Falling`): падение и установка блока при приземлении (`fx.land`). Если место занято непрозрачным блоком, выпадает предмет (`entity.rs`, около строк 436-445). Спавн из `interact.rs` и `/summon`. Нет: урона сущностям (в `entity.rs` урона сущностям нет вообще), NBT-полей `DropItem`, `HurtEntities`, `CancelDrop` (`summon.rs` читает только `BlockState` и `Time`). |
| `Interaction.java` | Interaction (взаимодействие) | НЕТ | Нет. Такой сущности в Rine нет, `/summon` отказывает. |
| `item/ItemEntity.java` | Предмет на земле | ЧАСТИЧНО | `game/entity.rs` (`Kind::Item`): гравитация, трение, таймер 6000 тиков, задержка подбора, слияние стопок (`merge_items`), подбор игроком (`pickup`), подбор воронкой (`blockinv.rs`, около строк 686-731). Нет: обработки воды и лавы (в `entity.rs` её нет), огня, владельца предмета. |
| `Display.java` | Предмет-дисплей | НЕТ | Нет. Сущностей отображения нет. |
| `LightningBolt.java` | Молния (эффект удара) | НЕТ | Сущности молнии нет: `LightningBolt` в Rine не найден. |
| `Marker.java` | Marker (маркер) | НЕТ | Нет. Такой сущности в Rine нет, `/summon` отказывает. |
| `OminousItemSpawner.java` | Спавнер зловещего предмета | НЕТ | Нет. Такой сущности в Rine нет, `/summon` отказывает. |
| `Display.java` | Текст-дисплей | НЕТ | Нет. Сущностей отображения нет. |
| `item/PrimedTnt.java` | Поджигаемый TNT | ЧАСТИЧНО | `game/entity.rs` (`Kind::Tnt`): запал, гравитация, сон на земле, взрыв через `explosion.rs` пачкой за тик. Спавн: `place.rs`, редстоун, `/summon`. Нет: `block_state` и `explosion_power` из NBT (`summon.rs` читает только `fuse`, блок всегда стандартный TNT). Не проверял поведение в воде и огне. |

#### 3. Снаряды (21)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `projectile/arrow/Arrow.java` | Стрела (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/hurtingprojectile/windcharge/BreezeWindCharge.java` | Заряд ветра бриза (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/hurtingprojectile/DragonFireball.java` | Огненный шар дракона (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/throwableitemprojectile/ThrownEgg.java` | Яйцо (снаряд) (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/throwableitemprojectile/ThrownEnderpearl.java` | Жемчуг Эндера (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/EvokerFangs.java` | Клыки заклинателя (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/throwableitemprojectile/ThrownExperienceBottle.java` | Склянка опыта (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/EyeOfEnder.java` | Око Эндера (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/hurtingprojectile/LargeFireball.java` | Большой огненный шар (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/FireworkRocketEntity.java` | Фейерверк (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/FishingHook.java` | Поплавок удочки | НЕТ | Нет. Удочки как снаряда нет. Таблица лута рыбалки в `itemcmd.rs` не загружается. |
| `projectile/throwableitemprojectile/ThrownLingeringPotion.java` | Стойкое зелье (бросок) (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/LlamaSpit.java` | Плевок ламы (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/ShulkerBullet.java` | Снаряд шалкера (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/hurtingprojectile/SmallFireball.java` | Маленький огненный шар (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/throwableitemprojectile/Snowball.java` | Снежок (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/arrow/SpectralArrow.java` | Светящаяся стрела (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/throwableitemprojectile/ThrownSplashPotion.java` | Взрывное зелье (бросок) (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/arrow/ThrownTrident.java` | Трезубец (бросок) (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/hurtingprojectile/windcharge/WindCharge.java` | Заряд ветра (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |
| `projectile/hurtingprojectile/WitherSkull.java` | Череп Иссушителя (снаряд) | НЕТ | Нет. Снарядов нет. Раздатчик бросает такой предмет как обычный (`blockinv.rs`, `drop_one`). |

#### 4. Транспорт (лодки, вагонетки) (29)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `vehicle/boat/Boat.java` | Лодка (акация) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (акация) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestRaft.java` | Плот с сундуком (бамбук) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Raft.java` | Плот (бамбук) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (берёза) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (берёза) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (вишня) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (вишня) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/minecart/MinecartChest.java` | Вагонетка с сундуком | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/minecart/MinecartCommandBlock.java` | Вагонетка с командным блоком | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (тёмный дуб) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (тёмный дуб) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/minecart/MinecartFurnace.java` | Вагонетка с печью | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/minecart/MinecartHopper.java` | Вагонетка с воронкой | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (джунгли) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (джунгли) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (мангровое дерево) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (мангровое дерево) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/minecart/Minecart.java` | Вагонетка | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (дуб) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (дуб) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (бледный дуб) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (бледный дуб) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (тополь) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (тополь) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/minecart/MinecartSpawner.java` | Вагонетка со спавнером | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/Boat.java` | Лодка (ель) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/boat/ChestBoat.java` | Лодка с сундуком (ель) | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |
| `vehicle/minecart/MinecartTNT.java` | Вагонетка с TNT | НЕТ | Нет. Транспорта нет: `/ride` отказывает (`cmd/commands/entities.rs`, `mount`). Лодка есть только как топливо печи (`containers.rs`, около строки 1029). |

#### 5. Декорации (7)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `decoration/ArmorStand.java` | Стойка для брони | НЕТ | Нет. Декоративных сущностей в Rine нет, `/summon` отказывает. |
| `decoration/Cushion.java` | Cushion (подушка) | НЕТ | Нет. Декоративных сущностей в Rine нет, `/summon` отказывает. |
| `decoration/GlowItemFrame.java` | Светящаяся рамка | НЕТ | Нет. Декоративных сущностей в Rine нет, `/summon` отказывает. |
| `decoration/ItemFrame.java` | Рамка | НЕТ | Нет. Декоративных сущностей в Rine нет, `/summon` отказывает. |
| `decoration/LeashFenceKnotEntity.java` | Узел поводка | НЕТ | Нет. Декоративных сущностей в Rine нет, `/summon` отказывает. |
| `decoration/Mannequin.java` | Манекен | НЕТ | Нет. Декоративных сущностей в Rine нет, `/summon` отказывает. |
| `decoration/painting/Painting.java` | Картина | НЕТ | Нет. Декоративных сущностей в Rine нет, `/summon` отказывает. Имя `painting_variant` уходит клиенту (`rine-data/src/registry.rs`), самих картин нет. |

#### 6a. Враждебные мобы (43)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `monster/Blaze.java` | Ифрит (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/skeleton/Bogged.java` | Bogged (болотный скелет) (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/breeze/Breeze.java` | Бриз (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/camel/CamelHusk.java` | Camel Husk (иссохший верблюд) (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/spider/CaveSpider.java` | Пещерный паук (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/creaking/Creaking.java` | Скрипун (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Creeper.java` | Крипер (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/zombie/Drowned.java` | Утопленник (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/ElderGuardian.java` | Древний страж (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Enderman.java` | Эндермен (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Endermite.java` | Эндермит (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/illager/Evoker.java` | Заклинатель (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Ghast.java` | Гаст (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Giant.java` | Великан (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Guardian.java` | Страж (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/hoglin/Hoglin.java` | Хоглин (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/zombie/Husk.java` | Иссохший зомби (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/illager/Illusioner.java` | Иллюзионист (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/cubemob/MagmaCube.java` | Магмовый куб (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/skeleton/Parched.java` | Parched (иссушенный) (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Phantom.java` | Фантом (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/piglin/Piglin.java` | Пиглин (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/piglin/PiglinBrute.java` | Пиглин-громила (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/illager/Pillager.java` | Разбойник (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Ravager.java` | Разоритель (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Shulker.java` | Шалкер (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Silverfish.java` | Чешуйница (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/skeleton/Skeleton.java` | Скелет (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/cubemob/Slime.java` | Слизень (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/spider/Spider.java` | Паук (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/skeleton/Stray.java` | Блуждающий (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/cubemob/SulfurCube.java` | Серный куб (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Vex.java` | Вредина (Vex) (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/illager/Vindicator.java` | Защитник (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/warden/Warden.java` | Смотритель (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Witch.java` | Ведьма (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/skeleton/WitherSkeleton.java` | Скелет Иссушителя (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Zoglin.java` | Зоглин (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/zombie/Zombie.java` | Зомби (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/equine/ZombieHorse.java` | Зомби-лошадь (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/nautilus/ZombieNautilus.java` | Зомби-наутилус (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). Реестр `zombie_nautilus_variant` уходит клиенту (`rine-data/src/registry.rs`), выбора варианта нет. |
| `monster/zombie/ZombieVillager.java` | Зомби-житель (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/zombie/ZombifiedPiglin.java` | Зомби-пиглин (monster) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |

#### 6b. Мирные наземные животные (29)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `animal/allay/Allay.java` | Аллей (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/armadillo/Armadillo.java` | Броненосец (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/bee/Bee.java` | Пчела (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/camel/Camel.java` | Верблюд (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/feline/Cat.java` | Кошка (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). Реестр `cat_variant` уходит клиенту (`rine-data/src/registry.rs`), выбора варианта нет. |
| `animal/chicken/Chicken.java` | Курица (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). Реестр `chicken_variant` уходит клиенту (`rine-data/src/registry.rs`), выбора варианта нет. |
| `animal/cow/Cow.java` | Корова (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). Реестр `cow_variant` уходит клиенту (`rine-data/src/registry.rs`), выбора варианта нет. |
| `animal/equine/Donkey.java` | Осёл (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/fox/Fox.java` | Лиса (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/frog/Frog.java` | Лягушка (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). Реестр `frog_variant` уходит клиенту (`rine-data/src/registry.rs`), выбора варианта нет. |
| `animal/goat/Goat.java` | Козёл (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/happyghast/HappyGhast.java` | Счастливый гаст (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/equine/Horse.java` | Лошадь (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/equine/Llama.java` | Лама (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/cow/MushroomCow.java` | Грибная корова (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/equine/Mule.java` | Мул (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/feline/Ocelot.java` | Оцелот (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/panda/Panda.java` | Панда (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/parrot/Parrot.java` | Попугай (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/pig/Pig.java` | Свинья (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). Реестр `pig_variant` уходит клиенту (`rine-data/src/registry.rs`), выбора варианта нет. |
| `animal/polarbear/PolarBear.java` | Белый медведь (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/rabbit/Rabbit.java` | Кролик (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/sheep/Sheep.java` | Овца (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/equine/SkeletonHorse.java` | Скелетная лошадь (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/sniffer/Sniffer.java` | Нюхач (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `monster/Strider.java` | Страйдер (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/frog/Tadpole.java` | Головастик (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/turtle/Turtle.java` | Черепаха (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/wolf/Wolf.java` | Волк (creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). Реестр `wolf_variant` уходит клиенту (`rine-data/src/registry.rs`), выбора варианта нет. |

#### 6c. Водные и летающие мирные (10)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `animal/axolotl/Axolotl.java` | Аксолотль (axolotls) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `ambient/Bat.java` | Летучая мышь (ambient) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/fish/Cod.java` | Треска (water_ambient) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/dolphin/Dolphin.java` | Дельфин (water_creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/squid/GlowSquid.java` | Светящийся спрут (underground_water_creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/nautilus/Nautilus.java` | Наутилус (water_creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/fish/Pufferfish.java` | Иглобрюх (water_ambient) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/fish/Salmon.java` | Лосось (water_ambient) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/squid/Squid.java` | Спрут (water_creature) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/fish/TropicalFish.java` | Тропическая рыба (water_ambient) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |

#### 7. Жители и торговцы (3)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `animal/equine/TraderLlama.java` | Лама торговца (житель или торговец) | НЕТ | Нет. Жителей, торговли, сплетен и точек работы не реализовано. |
| `npc/villager/Villager.java` | Житель (житель или торговец) | НЕТ | Нет. Жителей, торговли, сплетен и точек работы не реализовано. |
| `npc/wanderingtrader/WanderingTrader.java` | Бродячий торговец (житель или торговец) | НЕТ | Нет. Жителей, торговли, сплетен и точек работы не реализовано. |

#### 8. Боссы (2)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `boss/enderdragon/EnderDragon.java` | Дракон Края | НЕТ | Нет. Босса и Края не реализовано (`docs/HANDOFF.md`: Незер и Энд в планах). |
| `boss/wither/WitherBoss.java` | Иссушитель | НЕТ | Нет. Босса нет. |

#### 9. Големы (3)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| `animal/golem/CopperGolem.java` | Медный голем (misc) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/golem/IronGolem.java` | Железный голем (misc) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |
| `animal/golem/SnowGolem.java` | Снежный голем (misc) | НЕТ | Нет. Моба нет: ни сущности, ни здоровья, ни ИИ, спавна нет. `/summon` отказывает (`cmd/commands/summon.rs`). |

## Самое важное, чего нет

1. Мобы целиком: ни одного моба (зомби, скелеты, крипер, животные, жители, враждебные). Нет ни сущности, ни здоровья, ни ИИ. Это главная дыра.
2. Естественный спавн мобов и сущностей. Есть только `/summon` для трёх типов.
3. ИИ: цели, навигация, мозг, сенсоры (`ai/`, 277 файлов, кроме атрибутов игрока).
4. Снаряды: стрелы и луки, снежки, яйца, жемчуг Эндера, зелья, огненные шары, заряды ветра, фейерверки.
5. Урон и здоровье у сущностей, кроме игрока. Падающий блок никому не наносит урон.
6. Сферы опыта: опыт есть у игрока, но не выпадает сущностью.
7. Транспорт: лодки, вагонетки, `/ride` как рабочая команда.
8. Декорации: рамки, картины, стойки для брони, манекены, поводки.
9. Жители, торговля, бродячий торговец.
10. Боссы: дракон Края, иссушитель.
11. Вода и лава для предметов на земле (сейчас предмет в них ведёт себя как в воздухе). Флаги `NoGravity`, `Invulnerable`, `Glowing`, `Silent` сохраняются, но не действуют.
12. Молния, летучие мыши, рейды, деревни.

## Не уверен

- `ChatVisiblity`, `PlayerSkin` и части модели, `StackedContents` (учёт предметов для рецептов), `Crackiness` (для какой сущности класс).
- Поведение TNT в воде и огне, полное соответствие ванили у `FallingBlockEntity` при установке блока и выпадении предмета: построчно не сверял.

