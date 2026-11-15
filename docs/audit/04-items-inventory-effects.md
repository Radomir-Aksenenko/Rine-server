# Аудит 04: предметы, инвентарь, эффекты, атрибуты, очки (ванильный world, кроме level и entity)

Дата: 2026-11-15. Режим: только чтение кода. Код, коммиты и конфиги не менялись.

## Что проверялось

Ванила: `Paper-26.3/paper-server/src/minecraft/java/net/minecraft/world`, без `level` и `entity`, всего 570 файлов:
item 323, inventory 64, attribute 30, phys 28, файлы в корне world 25, effect 20, scores 16, ticks 14, damagesource 12, clock 11, waypoints 9, flag 7, food 6, timeline 5.

Rine: `crates/rine-server/src/game` (survival.rs, containers.rs, blockinv.rs, durability.rs, interact.rs, place.rs, fluids.rs, loot.rs, player.rs, cmd/commands/*), `crates/rine-data` (items.json, recipes.json), `crates/rine-command`.

Пути ниже даны от корня `crates/rine-server/src/game/`, если не сказано иначе.

## Сводка

- Работает основа. Стаки с количеством и износом, клики в окнах (инвентарь, верстак 2x2 и 3x3, сундуки, бочки, раздатчики, воронки, эндер-сундук, шалкеры, печи, коптильни, плавильни), крафт по shaped и shapeless рецептам, плавка, еда и голод, износ инструментов и брони, ведро с водой и лавой, TNT с огнивом, команды /effect, /attribute, /scoreboard, /team, /trigger, /waypoint, /tick, /time.
- Почти всё, что держится на компонентах предмета, отсутствует. У `player::Stack` есть только `item`, `count`, `damage` (player.rs:27-32). Поэтому нет имён, лора, зачарований, зелий, книг, карт, данных банок и т.п.
- Игровых эффектов от оружия и эффектов нет. Любой удар наносит 1.0 (interact.rs:456, 496), атрибуты оружия не учитываются. Эффекты зелий и еды хранятся в `cmd/commands/effect.rs` и истекают по времени, но не действуют на игрока. Щит только кладётся в левую руку и не блокирует урон.
- Снарядов нет. Нет стрел, лука, арбалета, трезубца, удочки, снежков, жемчуга, яиц, фейерверков. В `entity.rs:20-24` три вида сущностей: предмет, TNT, падающий блок.
- Меню не открываются для наковальни, зачарования, варочной стойки, точильного камня, ткацкого станка, картографического стола, кузни, камнереза, лектерна, маяка, крафтера, торговли с жителями (blockinv.rs:154-200 знает только верстак, печи, сундуки и их варианты).
- Рецепты читаются из `rine-data/assets/recipes.json` (2042 записи). Код (containers.rs:1165-1179) понимает 6 типов: shaped, shapeless, transmute, smelting, blasting, smoking. Это около 1342 записей. Не понимаются brewing (279), stonecutting (351), smithing (30), campfire_cooking (9), crafting_dye (6), 8 типов crafting_special (23 записи), decorated_pot и imbue.
- Пакет книги рецептов (C_UPDATE_RECIPES = 136, `rine-protocol/src/ids.rs:121`) нигде не отправляется. Клиент не получает список рецептов.
- Зачарования не моделируются. /enchant при наличии предмета отвечает ошибкой «не смоделировано» (cmd/commands/items.rs:280-310). Удача и шёлковое касание в луте не работают (loot.rs:4, 366).
- Торговли и жителей нет. Entity Kind не содержит жителей.

## Таблица по группам

Статусы: ЕСТЬ, ЧАСТИЧНО, НЕТ. Где «не уверен», там написано почему.

Покрытие: все 570 файлов учтены. Файлы, которых нет в таблице поштучно (компоненты, эффекты зачарований, провайдеры зачарований, рецепты-дисплеи, источники слотов, формы блоков), перечислены в приложении в конце с общим статусом. Поштучно такие классы не разбирались.

### item (ядро предметов и стаков)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| item/ItemStack, Items, Item, ItemInstance, ItemStackTemplate, ItemUtils, ItemProvider, ArrayItemProvider, ItemStackLinkedSet, AirItem | Стак: id, количество, максимум в стаке, износ, реестр предметов | ЧАСТИЧНО | player.rs:27-41 (`Stack` из item, count, damage), player.rs:112 `max_stack` из items.json, game/items.rs, crates/rine-data/src/items.rs. Компонентов нет. |
| item/component (53 файла) | Компоненты данных предмета (имя, лор, зачарования, кастомные данные, модель, инструмент, еда и т.д.) | НЕТ | Стак их не хранит. rine-command/src/args/item.rs разбирает SNBT компонентов, но до сервера они не доходят. cmd/commands/items.rs:4-6 это прямо говорит. |
| item/Rarity, TooltipFlag, ToolMaterial, ItemUseAnimation, SwingAnimationType, ItemDisplayContext, Instrument(s) | Перечисления и подписи предметов | НЕТ | Нужны в основном для компонентов и подсказок. Подсказки предмету не отправляются. |
| item/ItemCooldowns, ServerItemCooldowns | Перезарядка предметов (жемчуг, щит, еда и т.п.) | НЕТ | Нигде не найдено. Единственный «кулдаун» в коде это cooldown воронки (blockinv.rs:28). |
| item/CreativeModeTab, CreativeModeTabs | Вкладки креатива | НЕТ (не нужно на сервере) | Клиентская часть. Сервер принимает стек из креативного слота: durability.rs:162 `read_creative_stack`, handle.rs:134. |
| item/BlockItem, StandingAndWallBlockItem, DoubleHighBlockItem, PlaceOnWaterBlockItem, ScaffoldingBlockItem, GameMasterBlockItem, HangingSignItem (блоки) | Предмет ставит блок | ЧАСТИЧНО | items.rs `places` (default state блока), interact.rs:264-349 `use_item_on`, place.rs (`placement_state`). Верхняя половина двойных блоков: interact.rs:255 `other_half`. Вода для PlaceOnWater, лестницы и леса не проверял, не уверен. Подвесные таблички и стойки для брони не найдены. |
| item/HangingEntityItem, ItemFrameItem, ArmorStandItem, NameTagItem, LeadItem, BoatItem, MinecartItem, SpawnEggItem, EndCrystalItem | Предметы, которые создают сущности | НЕТ | Сущностей такого типа нет (entity.rs:20-24). В containers.rs:1029 лодка только как топливо (1200). |
| item/DyeItem, DyeColor, InkSacItem, GlowInkSacItem, HoneycombItem, SignApplicator, BoneMealItem | Краска, светящаяся краска, соты, рост костной мукой, покраска и воск | НЕТ | Не найдено (grep по bone_meal, dye, honeycomb, sign_applicator). Рецепт `crafting_dye` тоже не обрабатывается. |
| item/Items.java (реестр всех предметов) | Таблица предметов | ЕСТЬ | rine-data/assets/items.json, rine-data/src/items.rs. |

### item (инструменты, оружие, снаряжение, использование)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| item/BowItem, CrossbowItem, ArrowItem, SpectralArrowItem, TippedArrowItem, ProjectileWeaponItem, ProjectileItem | Лук, арбалет, стрелы, стрела-призрак, типпед-стрелы | НЕТ | Снарядов нет. survival.rs:328 `use_item` обрабатывает только еду. durability.rs:11 прямо пишет, что лук не учитывается. |
| item/TridentItem, MaceItem, WindChargeItem | Трезубец, булава, ветряной заряд | ЧАСТИЧНО | Только износ: durability.rs:27, 39 (`Family::Heavy`). Бросок трезубца, удар булавой и заряд не работают. |
| item/FishingRodItem | Удочка | НЕТ | Поплавка и вылова нет. durability.rs:11 это тоже признаёт. |
| item/FlintAndSteelItem, FireChargeItem | Огниво и огненный заряд | ЧАСТИЧНО | place.rs:402-438 `interact_block`: поджигают только TNT (поджиг блока огня не найден). Износ огнива: place.rs:421. |
| item/BucketItem, SolidBucketItem, MobBucketItem, DispensibleContainerItem | Ведро: набрать и вылить воду и лаву | ЧАСТИЧНО | fluids.rs:849 `use_bucket`, `pick_up`, `fill`. Есть вода и лава. Молоко, рыба, аксолотль, порошковый снег, снег в ведре: НЕТ. |
| item/ShieldItem, containers.rs:1007 (`shield` в левой руке) | Щит: блокирует урон, перезарядка, трещина | ЧАСТИЧНО | Щит можно положить в левую руку. Блокировки урона нет (в interact.rs не найдено). |
| Тотем бессмертия (предмет из Items.java, отдельного класса в item нет) | Воскрешение при смерти | НЕТ | В коде нет ни `totem`, ни события воскрешения. |
| Элитры (предмет из Items.java, слот брони в Equippable) | Полёт на элитрах | НЕТ | containers.rs:995 только кладёт элитры в слот брони. Полёта нет. |
| item/ShearsItem | Ножницы | ЧАСТИЧНО | Износ: durability.rs:29-30, 70, 80. Лут блоков с ножницами: loot.rs:28, 110-114. Стрижка овец и других сущностей: НЕТ. |
| item/FoodOnAStickItem | Удочка на морковке или грибе (управление седлом) | НЕТ | Сущностей для верховой езды нет. |
| item/BrushItem, Instrument, Instruments, InstrumentItem (козлиный рог) | Кисть, археология, козлиный рог | НЕТ | Не найдено. Блок нот есть в redstone (зона другого агента). |
| item/ExperienceBottleItem | Пузырёк опыта | НЕТ | Опыт не выпадает. См. cmd/commands/experience.rs (опыт хранится в отдельной карте по eid). |
| item/EnderpearlItem, EnderEyeItem, EggItem, SnowballItem, EndCrystalItem | Броски (жемчуг, глаз, яйцо, снежок) и кристалл | НЕТ | Снарядов и сущностей нет. |
| item/FireworkRocketItem | Фейерверк | НЕТ | Нет ни предмета, ни рецептов (crafting_special_firework_* не обрабатываются). |
| item/DebugStickItem | Палка отладки | НЕТ | Не найдено. |
| item/BundleItem | Пучок | НЕТ | Не найдено. |
| item/CompassItem, MapItem, EmptyMapItem, SpyglassItem | Компас, карта, пустая карта, подзорная труба | НЕТ | Не найдено. |
| item/WritableBookItem, WrittenBookItem, KnowledgeBookItem | Книги | НЕТ | Нет экрана книги и пакетов открытия. |
| item/JukeboxPlayable, JukeboxSong, JukeboxSongPlayer, JukeboxSongs | Пластинки в проигрывателе | НЕТ | blockent.rs:29, 49: проигрыватель только как тип блок-сущности с NBT. Проигрывания нет. |
| item/PotionItem, SplashPotionItem, LingeringPotionItem, ThrowablePotionItem, TippedArrowItem (зелья в предметах) | Зелья: пить, бросать, облако | НЕТ | Зелий нет в стаке (нет компонентов). |
| item/ServerItemCooldowns (см. выше) | Перезарядка | НЕТ | См. выше. |
| item/PlayerHeadItem, BannerItem | Голова игрока, знамя | НЕТ | Не найдено: узоры знамён и текстуры голов не хранятся. |

### item/equipment, item/slot, item/context (снаряжение и слоты)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| item/equipment/Equippable, EquipmentAsset(s), ArmorType, ArmorMaterial(s), AllowedEntitiesProvider | Слот брони, допустимые сущности, материалы | ЧАСТИЧНО | containers.rs:991-1005 `armor_slot` (слоты 5-8), containers.rs:1006 `offhand_item`. Материалы и модели брони не нужны на сервере, но их свойства (защита, прочность) не читаются. |
| item/equipment/trim/* (ArmorTrim, TrimMaterial(s), TrimPattern(s)) | Узоры брони | НЕТ | Рецепты `smithing_trim` (18 шт.) не обрабатываются. |
| броня: защита от урона (absorb, toughness, protection) | Снижение урона бронёй | НЕТ | В durability.rs:10-11 прямо сказано: броня не поглощает урон и toughness не учитывается. |
| броня: износ при ударе | Износ брони | ЕСТЬ | durability.rs:148 `hurt_armor`, но только для причин `player` и `explosion` (interact.rs:614). Огонь, падение и прочие причины износ брони не отнимают. Не уверен, какие из них в vanilla не снимают прочность (нужна проверка по DamageType). |
| item/consume_effects/* (ApplyStatusEffects, ClearAllStatusEffects, RemoveStatusEffects, PlaySound, TeleportRandomly) | Эффекты после еды и питья | НЕТ | survival.rs:524 `complete_meal` применяет только nutrition и saturation. Эффекты золотого яблока, суспа, сушёной ряски и подобных не применяются. Звук из компонента не используется отдельно. |
| item/context/BlockPlaceContext, DirectionalPlaceContext, UseOnContext | Контекст использования предмета на блок | ЧАСТИЧНО | Передаётся в place.rs `placement_state` (направление, курсор, поворот). Полного набора проверок нет. |
| item/slot/* (SlotSource, SlotSelector, SlotCollection и т.п.) | Выбор слотов для /item и /loot | ЧАСТИЧНО | cmd/commands/itemcmd.rs (слоты как у `Player.getSlot`). Для блоков: itemcmd.rs:310-316 читает и пишет содержимое через containers::block_items. Заголовок файла (строки 1-10) устарел: там написано, что доступа к контейнерам блоков ещё нет. |
| item/alchemy/* (Potion, PotionContents, PotionIds, Potions) | Зелья как компонент | НЕТ | См. выше, компонентов нет. |
| item/trading/* (Merchant, MerchantOffer(s), TradeSet(s), VillagerTrade(s), ItemCost, TradeCost, TradeRebalanceVillagerTrades) | Торговля жителей | НЕТ | Жителей нет. |
| item/enchantment/* (47 файлов: Enchantment, EnchantmentHelper, Enchantable, Repairable, ItemEnchantments, effects/*, providers/*) | Зачарования: уровни, эффекты, ремонт, зачарование столом | НЕТ | Зачарования не хранятся. /enchant отвечает ошибкой (cmd/commands/items.rs:280-310). Удача, шёлк, защита, прочность (Unbreaking) не учитываются (loot.rs:4, durability.rs:10). Зачаровательный стол как меню: НЕТ (blockent.rs:30 только тип блок-сущности). |
| item/enchantment/Repairable | Ремонт наковальней и предметом | НЕТ | См. выше. RepairItemRecipe (crafting_special_repairitem) не обрабатывается. |

### item/crafting (рецепты, верстаки, печи)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| item/crafting/ShapedRecipe, ShapedRecipePattern, ShapelessRecipe, NormalCraftingRecipe, CraftingRecipe, CraftingInput, RecipeHolder, RecipeMap, RecipeCache, RecipeManager | Крафт по рецепту в 2x2 и 3x3, отзеркаленные формы | ЕСТЬ | containers.rs:1198 `craft_result`, containers.rs:1133 `shaped_pattern`, containers.rs:1181 `recipes()` (читает recipes.json через rine-data). Ингредиенты-теги: containers.rs:1087-1118. Остаток от крафта (ведро из бутылки молока и т.п.): containers.rs:1011 `remainder`, только для ведёр. |
| item/crafting/TransmuteRecipe (crafting_transmute, 33 шт.) | Превращение предметов в сетке | ЕСТЬ | containers.rs:1170. |
| item/crafting/AbstractCookingRecipe, SmeltingRecipe, BlastingRecipe, SmokingRecipe | Плавка, плавильня, коптильня | ЕСТЬ | containers.rs:1276-1322 `smelting`, `smelt_for`; время с учётом blast и smoke. |
| item/crafting/CampfireCookingRecipe (9 шт.) | Жарка на костре | НЕТ | Тип не обрабатывается (containers.rs:1296-1299). Костёр как меню не открывается. |
| Топливо печи (ванильный источник лежит вне моей части, в level) | Время горения топлива | ЧАСТИЧНО | containers.rs:1019-1050 `fuel_of`: ручная таблица, не из тегов. Полнота не проверялась, не уверен, что совпадает с ванилой (например, все `*_boat`, `*_log`, `*_banner`, `wooden_*` записаны вручную). Опыт за плавку не выдаётся (не найдено). |
| item/crafting/StonecutterRecipe, SingleItemRecipe, SingleRecipeInput (stonecutting, 351 шт.) | Камнерез | НЕТ | Рецепты не обрабатываются, меню камнереза нет. |
| item/crafting/SmithingRecipe, SmithingTransformRecipe, SmithingTrimRecipe, SimpleSmithingRecipe, SmithingRecipeInput, item/SmithingTemplateItem | Кузнечный стол: улучшение (12), узоры (18) | НЕТ | Рецепты не обрабатываются. Меню нет. |
| item/crafting/BrewingRecipe, BrewingInput, PotionIngredient (brewing, 279 шт.) | Варка зелий | НЕТ | Рецепты не обрабатываются. Варочной стойки как меню нет (blockent.rs:29 только NBT). |
| item/crafting/ImbueRecipe, DecoratedPotRecipe (crafting_imbue, crafting_decorated_pot) | Пропитка, украшенный горшок | НЕТ | Рецепты не обрабатываются. |
| item/crafting/DyeRecipe (crafting_dye, 6 шт.), BannerDuplicateRecipe, BookCloningRecipe, FireworkRocketRecipe, FireworkStarRecipe, FireworkStarFadeRecipe, MapExtendingRecipe, RepairItemRecipe, ShieldDecorationRecipe, CustomRecipe | Специальные рецепты (покраска брони, знамёна, книги, фейерверки, карты, ремонт, узор щита) | НЕТ | В recipes.json 23 записи crafting_special_* (8 типов) и 6 crafting_dye. build_recipe (containers.rs:1171) возвращает None для всех. |
| item/crafting/Recipe, RecipeInput, RecipeSerializers, RecipeCraftingHolder, StackedContentsCompatible | Общие интерфейсы рецепта и входа | ЧАСТИЧНО | Своих интерфейсов нет, есть разбор в containers.rs:1087-1200. Заметки о сборке стека (StackedContents) для книги рецептов не нужны без книги. |
| item/crafting/TransmuteResult | Результат превращения (предмет, количество, компоненты) | ЧАСТИЧНО | containers.rs:1170 берёт только предмет и количество из рецепта. Компоненты результата не переносятся (их и нет в стаке). |
| item/BottleItem | Пустая бутылка: набрать воду, получить драконье дыхание | НЕТ | Поиска glass_bottle и water_bottle в server-коде нет. Наполнение бутылки не работает. |
| item/CushionItem | Подушка (сущность cushion при установке) | НЕТ | Сущности cushion нет (entity.rs:20-24). |
| item/DiscFragmentItem | Осколок пластинки | НЕТ | Не найдено. |
| item/AdventureModePredicate | Предикаты «можно ломать» и «можно ставить» в приключенческом режиме | НЕТ | Приключенческий режим есть (mod.rs:116, durability.rs:85), предикатов can_break и can_place нет. blockinv.rs:492 `can_place` относится к хранилищам, это другая функция. |
| item/crafting/RecipeBookCategory, CraftingBookCategory, CookingBookCategory, ExtendedRecipeBookCategory, RecipeBookCategories, RecipeDisplay* (display/*), SlotDisplay*, SelectableRecipe, PlacementInfo, RecipeAccess | Книга рецептов и её отображение | НЕТ | Пакеты книги рецептов не отправляются (ids.rs:121 есть, в коде не используется). Книги рецептов у клиента нет. |
| item/crafting/RecipeSerializer(s), RecipeType, Ingredient, RecipePropertySet | Разбор рецептов из данных | ЧАСТИЧНО | containers.rs:1087-1119 `resolve`, `collect_items`, `parse_result`. Своего загрузчика нет, он читает готовый recipes.json. |

### inventory (меню, контейнеры, клики)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| inventory/InventoryMenu, AbstractContainerMenu, Slot, SlotRange(s), ClickAction, ContainerInput, ArmorSlot, ResultSlot, NonInteractiveResultSlot, RemoteSlot, ContainerSynchronizer, ContainerListener, ContainerData, DataSlot, SimpleContainerData | Окно, слоты, клики, синхронизация, данные окна | ЕСТЬ | containers.rs:34 `click`, containers.rs:1329 `apply_click` (режимы клика, перетаскивание, быстрый перенос, обмен, выброс, quick craft), containers.rs:1793 `refresh`, containers.rs:204 id меню через реестр. Полного покрытия всех режимов не проверял, не уверен. |
| inventory/CraftingMenu, CraftingContainer, TransientCraftingContainer, AbstractCraftingMenu, RecipeBookMenu | Верстак 3x3 и инвентарь 2x2 | ЕСТЬ | blockinv.rs:154 `crafting_table`, Lay::Table и Lay::Inv (containers.rs:381-398). Книга рецептов: НЕТ. |
| inventory/ChestMenu, ShulkerBoxMenu, ShulkerBoxSlot, DispenserMenu, HopperMenu, PlayerEnderChestContainer | Сундук, двойной сундук, шалкер, раздатчик, раздатчик с падением, воронка, эндер-сундук | ЕСТЬ | blockinv.rs:158-200, containers.rs:1997 `ender_items`. Шалкер: проверка крышки (blockinv.rs:169-178), шалкер не кладётся в шалкер (containers.rs:1007 и Store.shulker). |
| inventory/AbstractFurnaceMenu, FurnaceMenu, BlastFurnaceMenu, SmokerMenu, FurnaceFuelSlot, FurnaceResultSlot | Печь, плавильня, коптильня | ЕСТЬ | blockinv.rs:72-77, containers.rs:280 `furnace_tick`, containers.rs:932 `furnace_data`. Опыт за результат: НЕТ. |
| inventory/AnvilMenu, ItemCombinerMenu, ItemCombinerMenuSlotDefinition | Наковальня | НЕТ | Меню нет, есть только установка блока (place.rs:141) и падение (interact.rs:242). |
| inventory/EnchantmentMenu | Зачаровательный стол | НЕТ | См. enchantment. |
| inventory/BrewingStandMenu | Варочная стойка | НЕТ | См. brewing. |
| inventory/GrindstoneMenu | Точильный камень | НЕТ | Блок ставится и крепится (neighbors.rs:158), меню нет. |
| inventory/LoomMenu | Ткацкий станок | НЕТ | Блок есть в place.rs:139 и redstone (engine.rs:1920), меню нет. |
| inventory/CartographyTableMenu | Картографический стол | НЕТ | Не найдено. |
| inventory/SmithingMenu | Кузнечный стол | НЕТ | См. smithing. |
| inventory/StonecutterMenu | Камнерез | НЕТ | Блок ставится (place.rs:139), меню нет. |
| inventory/LecternMenu | Лекторн | НЕТ | Блок ставится (place.rs:139), меню и книги нет. |
| inventory/BeaconMenu | Маяк | НЕТ | Только тип блок-сущности (blockent.rs:30). |
| inventory/CrafterMenu, CrafterSlot | Крафтер | НЕТ | Блок упомянут в place.rs:139. Меню нет. |
| inventory/MerchantMenu, MerchantContainer, MerchantResultSlot | Торговля | НЕТ | См. trading. |
| inventory/HorseInventoryMenu, AbstractMountInventoryMenu, NautilusInventoryMenu | Инвентарь лошади и подобных | НЕТ | Сущностей нет. |
| inventory/ResultContainer, ContainerLevelAccess, RecipeBookType, RecipeCraftingHolder | Результат крафта, доступ к блоку окна, тип книги рецептов | ЧАСТИЧНО | containers.rs:1198 (результат крафта считается при каждом клике), блок окна проверяется в containers.rs:171 `open_block`. Книги рецептов нет. |
| inventory/MenuType, MenuConstructor, MenuProvider (корень: SimpleMenuProvider, MenuProvider) | Типы меню и их создание | ЧАСТИЧНО | blockinv.rs:144-200 `menu_for` и containers.rs:171 `open_block`: создание окна зашито в код по имени блока, без общей системы. |
| inventory/tooltip/* | Подсказки предметов | НЕТ | Не отправляются. |
| inventory/ItemStackWithSlot (корень world) | Стек со слотом | ЧАСТИЧНО | Внутренние структуры containers.rs, отдельного типа нет. |

### world (корень: контейнеры, сложность, таймеры, прочее)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| Container, WorldlyContainer, WorldlyContainerHolder, SimpleContainer, CompoundContainer, ContainerHelper, Containers, Clearable | Модель контейнера, грани доступа, двойной сундук, очистка | ЧАСТИЧНО | Логика есть, но не в виде общих классов: blockinv.rs:399-405 `Ct`, blockinv.rs:473-510 (грани для печи и хоппера), containers.rs:698-782 (загрузка и сохранение содержимого). |
| RandomizableContainer, LockCode | Лут в сундуках и замок сундука | НЕТ | Лут контейнеров не найден (loot.rs описывает только лут блоков). Замок по коду не найден. |
| Nameable | Имя контейнера | ЧАСТИЧНО | blockinv.rs:136-146 `custom_name`, `title`, читает CustomName из NBT. |
| RandomSequence, RandomSequences | Именованные последовательности /random | ЧАСТИЧНО | cmd/commands/random.rs (Xoroshiro, MD5 из id). Не сохраняются в `random_sequences.dat` (это записано в заголовке файла). |
| Stopwatch, Stopwatches | Секундомеры | ЧАСТИЧНО | cmd/commands/stopwatch.rs. Только в памяти, сохранения нет (заголовок файла). |
| TickRateManager | Скорость тика, заморозка, шаг, спринт | ЕСТЬ | cmd/commands/world.rs:168-240 (`rate`, `frozen`, `step_left`, `sprint_left`), world.rs:223 `tick_should_run`, команда /tick (world.rs:49, 71). |
| Difficulty, DifficultyInstance | Сложность и региональная сложность | ЧАСТИЧНО | Команда /difficulty есть (world.rs:49). Региональная сложность (DifficultyInstance) не найдена. Влияние сложности на урон не проверял, не уверен. |
| InteractionHand, InteractionResult | Рука и результат взаимодействия | ЕСТЬ | Рука: handle.rs и interact.rs (`hand == 1` = левая). Результат: в коде нет отдельного типа, решения принимаются через возвраты функций. |
| BossEvent | Босс-бар | ЕСТЬ | cmd/commands/bossbar.rs (566 строк). Сам класс BossEvent детально не сверял, не уверен, полное ли покрытие. |
| WorldlyContainer (в blockinv) | Грани доступа к хоппер/печь | ЕСТЬ | См. выше. |

### scores (скорборд, команды)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| scores/Scoreboard, Objective, Score, ScoreAccess, ScoreHolder, PlayerScoreEntry, PlayerScores, ReadOnlyScoreInfo, DisplaySlot, ScoreboardSaveData | Цели, очки, слоты показа, сохранение | ЕСТЬ | cmd/commands/scoreboard.rs (2397 строк): /scoreboard objectives, players, display. Сохранение scoreboard.dat в ванильном формате (заголовок файла). |
| scores/PlayerTeam, Team, TeamColor | Команды: цвет, friendly fire, видимость ника | ЕСТЬ | scoreboard.rs:130-149 (все ветки /team modify). |
| criteria/ObjectiveCriteria | Критерии очков | ЧАСТИЧНО | scoreboard.rs:226-232 `criteria_of`: dummy, trigger, health, food, air, armor, xp, level (только чтение). Автоподсчёт по событиям (deathCount, playerKillCount, mobKillCount, stat.*) не найден. |
| Слот показа в ванили (отображение сердец, формат чисел) | Формат очков | ЕСТЬ | scoreboard.rs:83-84 (`integer`, `hearts`), scoreboard.rs:191 (формат). |
| /trigger | Триггеры игроков | ЕСТЬ | scoreboard.rs:159-161, 1838. |

### effect (эффекты)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| effect/MobEffectInstance, MobEffect, MobEffects, MobEffectUtil, MobEffectCategory | Эффект: id, уровень, длительность, флаги, истечение | ЧАСТИЧНО | cmd/commands/effect.rs:26-40 (struct Effect), effect.rs:117-160 (/effect give и clear), effect.rs:192 `tick` (истечение, отправка remove). Флаги ambient, visible, show_icon сохраняются. |
| effect/PoisonMobEffect, RegenerationMobEffect, WitherMobEffect, HungerMobEffect, SaturationMobEffect, AbsorptionMobEffect, HealOrHarmMobEffect, InstantaneousMobEffect | Поведение эффектов: яд, регенерация, иссушение, голод, насыщение, поглощение, мгновенный урон и лечение | НЕТ | Эффект не действует на игрока (заголовок effect.rs:4-8). В survival.rs эффекты не читаются. |
| effect/InfestedMobEffect, OozingMobEffect, WeavingMobEffect, WindChargedMobEffect, BadOmenMobEffect, RaidOmenMobEffect | Эффекты мобов и рейдов | НЕТ | Мобов нет. |
| effect (пакеты обновления, C_UPDATE_MOB_EFFECT 135, C_REMOVE_MOB_EFFECT 79) | Синхронизация эффекта с клиентом | ЕСТЬ | effect.rs:23-24 (константы id пакетов), effect.rs:213 (отправка remove) и выдача эффекта. |
| effect: эффекты зелий в предметах (зелья, еда) | Выдача эффекта от предмета | НЕТ | См. consume_effects и alchemy. |

### attribute (атрибуты)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| attribute/AttributeType, AttributeTypes, AttributeRange | Атрибуты: типы, диапазоны | ЧАСТИЧНО | cmd/commands/effect.rs:43-181: /attribute get, base get/set/reset, modifier add/remove/get. Диапазоны и реестр типов в коде не сверял, не уверен. |
| attribute/modifier/AttributeModifier, FloatModifier, IntegerModifier, BooleanModifier, ColorModifier, ListModifier, FloatWithAlpha, MobSpawnSettingsModifier | Модификаторы атрибутов | ЧАСТИЧНО | effect.rs:170-175 (add_value, add_multiplied_base, add_multiplied_total). Значения не влияют на геймплей (заголовок effect.rs:7-8). |
| Атрибуты игрока на геймплей (скорость ходьбы, макс. здоровье, урон оружия, сила удара) | Влияние атрибутов | НЕТ | Скорость, здоровье и урон зашиты в код (interact.rs:456 ATTACK_DAMAGE, survival.rs:19 MAX_HEALTH). В HANDOFF.md пункт 7 «Атрибуты оружия (сейчас всё бьёт как кулак)». |
| attribute/EnvironmentAttribute*, EnvironmentAttributeMap, EnvironmentAttributeProbe, EnvironmentAttributeReader, EnvironmentAttributeSystem, EnvironmentAttributeLayer, SpatialAttributeInterpolator, WeatherAttributes, BedRule, AmbientSounds, AmbientMoodSettings, AmbientParticle, AmbientAdditionsSettings, BackgroundMusic, LerpFunction, GaussianSampler | Атрибуты среды: цвет неба, туман, звуки, музыка, правила сна | НЕТ | Не найдено. В ids.rs нет пакетов среды, кроме C_SET_TIME (ids.rs:111) и C_UPDATE_ATTRIBUTES (ids.rs:120, это атрибуты сущностей). |

### damagesource (источники урона)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| damagesource/DamageSource, DamageSources, DamageType, DamageTypes | Источник урона и его тип в реестре | ЧАСТИЧНО | interact.rs:567-579 `damage_type_id` (поиск в реестре с запасным generic), interact.rs:581-600 `damage` и `hurt`. Причины: `player`, `explosion`, `void` и другие строки. |
| damagesource/CombatRules, DamageScaling | Формула брони и масштаб по сложности | НЕТ | Снижения урона бронёй нет (см. durability.rs:10-11). |
| damagesource/CombatTracker, CombatEntry, FallLocation, DeathMessageType | Журнал боя и сообщения о смерти | не уверен | Не нашёл отдельного журнала боя. Сообщения о смерти по типу урона в коде не искал, поэтому статус не уверен. |
| damagesource/DamageEffects | Звук и отбрасывание по типу урона | ЧАСТИЧНО | interact.rs:620-627 (`damage_event` для клиента), отбрасывание interact.rs:553 `knock`. Полного соответствия типам не проверял. |
| Падение | Урон от падения | ЕСТЬ | survival.rs:568 `fall_damage`, survival.rs:598-660 `check_fall`, `land`, `cause_fall`. |

### food (еда и голод)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| food/FoodData, FoodConstants | Голод, насыщение, истощение, регенерация | ЕСТЬ | survival.rs:18-70 (константы), survival.rs:74-135 (`Stats`, `food_tick`, `apply_food`), survival.rs:89 `add_exhaustion`. |
| food/FoodProperties, Foods | Свойства еды: питание, насыщение, можно есть всегда | ЧАСТИЧНО | survival.rs:190-203 `food_of` (nutrition, saturation, canAlwaysEat из items.json). Эффекты еды не применяются. |
| Время еды | Длительность поедания | ЧАСТИЧНО | survival.rs:205-211 `eat_ticks`: 32 тика, сушёная ряска 16. Не из данных, таблица вручную. Не уверен, что совпадает со всеми предметами. |
| Остаток после еды (миска, бутылка, ведро молока) | Возврат предмета | НЕТ | survival.rs:538-547: уменьшается только count. Суп и мёд не возвращают миску и бутылку. |
| Поедание в `use_item` | Старт и отмена | ЕСТЬ | survival.rs:328-371 `use_item`, `release_use_item`. Заглушка interact.rs:366 `use_item` в handle.rs не вызывается. |
| food/VillagerFood | Еда жителей | НЕТ | Жителей нет. |

### phys (векторы, AABB, фигуры, попадания)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| phys/Vec3, Vec2, AABB | Векторы и коробки | ЧАСТИЧНО | Векторы записаны как массивы `[f64; 3]`. Отдельных типов Vec3, Vec2 и AABB не нашёл (поиск по `struct Vec3` и `struct Aabb` пуст). |
| phys/HitResult, BlockHitResult, EntityHitResult | Результат луча | ЧАСТИЧНО | fluids.rs `raycast_with` (для ведра), interact.rs:350 `intersects_player`. Попадание по сущностям нет (сущностей нет). |
| phys/shapes/* (VoxelShape, Shapes, CollisionContext, merges, DiscreteVoxelShape, CubeVoxelShape и др.) | Формы блоков для коллизий | ЧАСТИЧНО | crates/rine-data/src/blocks.rs: поле `shape` (индекс) и `shapes` (списки AABB в координатах блока, `Vec<Vec<[f32; 6]>>`), флаг FULL_CUBE. Данные есть, как их используют движение и луч, проверил не полностью. Контекста коллизий (CollisionContext, для предметов и вагонеток) и булевых операций над формами (BooleanOp, merges) не нашёл. |
| phys/BlockHitResult по форме блока при установке (CushionItem, UseOnContext) | Точка попадания на форму блока | не уверен | Не проверял, есть ли у установки блока точное попадание по форме, см. interact.rs:264 `use_item_on`. |

### clock и timeline (часы мира и время суток)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| clock/WorldClock, WorldClocks, ClockManager, ServerClockManager, ClockInstance, ClockState, ClockNetworkState, ClockTimeMarker(s), PackedClockStates | Мировые часы, их скорость, пауза, отправка клиенту | ЧАСТИЧНО | cmd/commands/world.rs:187-210 (`Clock`, `advance_clock`, `broadcast_clock`), команда /time (world.rs:49). Несколько часов, маркеры и сохранение состояния часов не найдены (не уверен). |
| timeline/Timeline, Timelines | Временные шкалы (день, маркеры) | ЧАСТИЧНО | cmd/commands/world.rs:28 (маркеры из timeline/day.json). Общей системы временных шкал нет. |
| timeline/AttributeTrack, AttributeTrackSampler | Изменение атрибутов среды по времени | НЕТ | См. attribute, атрибуты среды. |

### ticks (отложенные тики блоков и жидкостей, приоритет)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| ticks/ScheduledTick, TickPriority, SavedTick, SerializableTickContainer, LevelTicks, LevelChunkTicks, ProtoChunkTicks, TickAccess, TickContainerAccess, WorldGenTickAccess, BlackholeTickAccess, ContainerSingleItem, LevelTickAccess | Очередь отложенных тиков блоков и жидкостей, приоритеты, сохранение | ЧАСТИЧНО | redstone/mod.rs:102 `schedule_tick` (с задержкой и приоритетом, аналог `scheduleTick`), fluids.rs (планирование жидкостей). Сохранение очереди в чанке и общие классы ticks не нашёл. Эта очередь тесно связана с уровнем мира (зона другого агента), не уверен в полноте. |
| ContainerSingleItem | Одиночный контейнер для тиков | НЕТ | Не найден отдельного аналога. |

### flag (флаги функций)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| flag/FeatureFlag, FeatureFlagSet, FeatureFlagRegistry, FeatureFlagUniverse, FeatureFlags, FeatureElement | Наборы флагов функций (экспериментальные функции) | НЕТ | Не найдено. Для обычной игры набор по умолчанию, отдельного механизма обычно не требуется. Не уверен, нужно ли для протокола 777 и реестров. |

### waypoints (вэйпоинты, метки на карте клиента)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| waypoints/Waypoint, WaypointManager, TrackedWaypoint, TrackedWaypointManager, WaypointTransmitter, WaypointStyleAsset(s), PartialTickSupplier | Метки игроков и их стиль, отслеживание | ЧАСТИЧНО | cmd/commands/misc/waypoint.rs (216 строк). Передатчики только игроки (заголовок файла): других живых сущностей нет. Цвет и стиль хранятся по UUID игрока, смена отправляется как UNTRACK и TRACK. |

## Что важнее всего добавить (что нет совсем, но нужно большинству игроков)

1. Компоненты предмета в `player::Stack`. От них зависят зачарования, зелья, книги, карты, имена, лор и почти вся ванильная логика предметов. Это главный блокер.
2. Зачарования: применение в инструментах, броне и оружии, зачаровательный стол, наковальня, ремонт.
3. Снаряды: стрелы, лук, арбалет, трезубец, жемчуг, снежки, яйца, фейерверки. Для этого нужны сущности снарядов (сейчас в entity.rs только предмет, TNT и падающий блок).
4. Атрибуты оружия и брони: удар по-разному для мечей, топоров, булавы, защита брони в формуле урона (сейчас урон фиксирован, броня не поглощает).
5. Эффекты зелий и еды: регенерация, яд, иссушение, голод, насыщение, поглощение и т.д. Сейчас эффекты только хранятся.
6. Щит (блокировка урона), тотем бессмертия, элитры (полёт).
7. Рецепты: brewing, stonecutting, smithing, campfire_cooking, покраска, специальные (фейерверки, знамёна, карты, книги, ремонт). Отправка рецептов клиенту (книга рецептов).
8. Меню: наковальня, зачаровательный стол, варочная стойка, точильный камень, ткацкий станок, камнерез, кузнечный стол, картографический стол, лекторн, маяк, крафтер.
9. Опыт: выпадение опыта (руда, убийства, плавка) и опыт в зачаровании.
10. Торговля жителей и жители (вместе с сущностями, это зона другого агента).
11. Ведро: молоко и рыба; огниво для огня; кость для роста растений; мотыги, лопаты, топоры (вспашка, снятие коры).
12. Перезарядка предметов (ServerItemCooldowns).

## Заметки

- Файлы в обоих списках (ItemStackWithSlot, SlotCollection и др.) проверены только по названию и ключевым вызовам, не по построчной логике. Полное соответствие механик не проверялось.
- Заголовок cmd/commands/itemcmd.rs (строки 1-10) говорит, что доступа к контейнерам блоков ещё нет. В коде `itemcmd.rs:310-316` уже вызываются `containers::block_items` и `set_block_items`. Комментарий устарел.
- Заглушка `interact::use_item` (interact.rs:366) не вызывается из handle.rs. Настоящая обработка еды в survival.rs.
- fuel_of (containers.rs:1019) и eat_ticks (survival.rs:205) заданы вручную, не из данных. Их стоит сверить с ванилой.

## Приложение: классы без отдельной строки

Статус общий для группы. Поштучно не разбирались.

- НЕТ, компоненты предмета (item/component, все не поддерживаются, стак их не хранит): AttackRange, Bees, BlockItemStateProperties, BlocksAttacks, BookContent, BundleContents, BundleTooltip, ChargedProjectiles, Compostable, Consumable, Consumables, ConsumableListener, ConsumeEffect, ContainerComponent, CustomData, CustomModelData, DeathProtection, DebugStickState, DyedItemColor, Fireworks, FireworkExplosion, InstrumentComponent, ItemAttributeModifiers, ItemContainerContents, ItemLore, KineticWeapon, LodestoneTracker, MapDecorations, MapPostProcessing, OminousBottleAmplifier, PiercingWeapon, ProvidesTrimMaterial, ResolvableProfile, SuspiciousStewEffects, TooltipComponent, TooltipDisplay, TooltipProvider, TypedEntityData, UseCooldown, UseEffects, UseRemainder, WritableBookContent, WrittenBookContent, Ignite (кроме огня от TNT), SulfurCubeContent.
- НЕТ, эффекты зачарований и провайдеры: AllOf, AddValue, ApplyEntityImpulse, ApplyExhaustion, ApplyMobEffect, ChangeItemDamage, ConditionalEffect, DamageEntity, DamageImmunity, DamageResistant, EnchantedItemInUse, EnchantmentAttributeEffect, EnchantmentEffectComponents, EnchantmentEntityEffect, EnchantmentInstance, EnchantmentLocationBasedEffect, EnchantmentProvider, EnchantmentProviderTypes, EnchantmentTarget, EnchantmentValueEffect, EnchantmentsByCost, EnchantmentsByCostWithDifficulty, ExplodeEffect, LevelBasedValue, MobVisibility, MultiplyValue, PlaySoundEffect, RemoveBinomial, ReplaceBlock, ReplaceDisk, RunFunction, ScaleExponentially, SetBlockProperties, SetValue, SingleEnchantment, SpawnParticlesEffect, SummonEntityEffect, TargetedConditionalEffect, VanillaEnchantmentProviders.
- НЕТ, предметные эффекты еды: ApplyStatusEffectsConsumeEffect, ClearAllStatusEffectsConsumeEffect, PlaySoundConsumeEffect, RemoveStatusEffectsConsumeEffect, TeleportRandomlyConsumeEffect.
- НЕТ, торговля, тримы, модели брони и карты: MerchantOffers, TradeSets, TrimMaterials, TrimPatterns, ArmorMaterials, EquipmentAssets, SeededContainerLoot, GrowableMutableContainer, SimpleMutableContainer.
- НЕТ, дисплеи рецептов (для книги рецептов): RecipeDisplayEntry, RecipeDisplayId, RecipeDisplays, ShapedCraftingRecipeDisplay, ShapelessCraftingRecipeDisplay, FurnaceRecipeDisplay, SmithingRecipeDisplay, StonecutterRecipeDisplay, DisplayContentsFactory, SlotDisplays, SlotDisplayContext.
- НЕТ, топливо алхимии и печи: BrewingFuel, CookingFuel.
- ЧАСТИЧНО, слоты (cmd/commands/itemcmd.rs): CompositeSlotSource, ContentsSlotSource, CountingModifier, EmptySlotSource, FilteredSlotSource, GroupSlotSource, LimitSlotSource, RangeSlotSource, SlotRanges, SlotSources, TransformedSlotSource.
- ЧАСТИЧНО, часы и окружение: ClockTimeMarkers (см. clock), EnvironmentAttributes (НЕТ, см. attribute), BlockTransformers, OffsetDoubleList.
- ЧАСТИЧНО или не уверен, формы блоков (phys/shapes): ArrayVoxelShape, BitSetDiscreteVoxelShape, CubePointRange, DiscreteCubeMerger, EntityCollisionContext, IdenticalMerger, IndexMerger, IndirectMerger, MinecartCollisionContext, NonOverlappingMerger, PositionCollisionContext, SliceShape, SubShape. Общего модуля форм нет, данные коллизий есть в rine-data (см. выше).
- НЕТ, другое: CushionItem уже в таблице, BrewingFuel и CookingFuel (см. выше), DebugStickState (палка отладки, НЕТ).
