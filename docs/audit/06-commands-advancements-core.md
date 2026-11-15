# Аудит 06: команды, аргументы, достижения, core, nbt и остальная ваниль

Дата: 2026-11-15. Ветка `commands`. Код не менялся, ничего не коммитилось.

## Сводка

- Все 1416 исполняемых путей дерева команд (`crates/rine-command/assets/tree.json`) имеют исполнитель: `cargo test -p rine-server registry_paths_valid` дал `executors: 1416/1416`. Но исполнитель не значит «работает»: часть путей отвечает заглушкой (`stub`, `unmodelled`, «Rine: ... is not implemented yet») или пустым набором.
- Из 92 верхних узлов дерева (с алиасами): ЕСТЬ у 58, ЧАСТИЧНО у 27, НЕТ у 7. Алиасы (`tell`, `w`, `tm`, `tp`, `xp`) считаются отдельно. Таблица ниже.
- Все 62 парсера аргументов из `minecraft:command_argument_type` разбираются (`rine-command/src/args/mod.rs`, `from_parser` и `parse`). Парсинг почти полный, а вот исполнение аргументов часто ограничено (предметы без компонентов, лут только блочный, плейсхолдеры для структур).
- Достижения (`advancements/`, 115 файлов): в Rine нет. Команда `/advancement` и селектор `advancements=` отвечают как при пустом реестре, фильтр не совпадает ни с кем.
- Статистика (`stats/`) и книга рецептов клиента (`recipebook/`): нет. Крафт есть (`containers.rs`, `recipes.json`), но без разблокировки рецептов.
- Компоненты предметов (`core/component/`, ~40 файлов): хранится только `minecraft:damage`. Из-за этого не работают `/enchant`, компоненты в `/give`, предикаты с компонентами, зачарования (Unbreaking, Fortune, Silk Touch).
- Tab-подсказки: сервер не отвечает на запрос подсказок клиента (`S_COMMAND_SUGGESTION` не обрабатывается). Для аргументов с `ask_server` (id, функции, звуки и т.п.) Tab ничего не подскажет.
- Критерии скоринборда на статистику и смерти (`deathCount`, `playerKillCount`, `minecraft.*:*`) принимаются командой, но не растут: игра в скоринборд не пишет вообще, кроме команд.
- Эффекты зелий и атрибуты: по комментарию в `effect.rs` (строка 191) эффекты с серверным поведением (лечение, урон, скорость, сила, прочность и т.п.) существуют только как клиентские. То есть `/effect` ведёт список и таймер, но не меняет геймплей. Проверил комментарий, в код не лез дальше.
- Мобов нет. Все команды, которые в ванили работают с сущностями, в Rine работают с игроками, предметами, TNT и падающими блоками. Это касается `kill`, `damage`, `swing`, `ride`, `data entity`, `summon`, `attribute`.
- Измерений Незер и Энд нет (`docs/HANDOFF.md`, пункт 7). Аргумент `execute in` их парсит, но миров нет.
- Правила игры: все 58 правил из ванили есть в `rine-server/src/game/cmd/rules.rs`, имена совпали с деревом один в один.

### Методика

- Список команд: верхний уровень `tree.json` (92 узла) и `V/server/commands/*Command*.java` плюс `V/commands/Commands.java` (блок `register`). Классы, которые в релизе не регистрируются (dev-флаги, integrated), отмечены отдельно.
- Статусы по шапкам модулей Rine (`//!` строки), по заглушкам (`stub`, `unmodelled`, `unimplemented`) и по чтению основных исполнителей. Построчно сверил не все команды. Где поведение не смотрел, статус помечен как «не уверен».
- Тесты и сборку не запускал, кроме `registry_paths_valid` (только чтение тестового вывода).

Легенда статусов: ЕСТЬ (механика работает как в ванили, с оговорками, если они указаны), ЧАСТИЧНО (часть веток или сущностей работает), НЕТ (механики нет, либо команда отвечает ошибкой или пустым набором), НЕ НУЖНО (генераторы и инфраструктура, рантайму сервера не нужны), N/A (в релизном сервере не регистрируется).

## Команды (верхний уровень дерева `tree.json`)

Исполнители лежат в `crates/rine-server/src/game/cmd/commands/`. Путь «misc» это `commands/misc/`.

| Команда | Ванильный класс | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| advancement | AdvancementCommands | НЕТ | `commands/misc/advancement.rs`. Имя достижения всегда «не найдено», `everything` ничего не меняет. Дерева и прогресса нет |
| attribute | AttributeCommand | ЧАСТИЧНО | `commands/effect.rs`. Значения, база и модификаторы есть, только для игроков. Мобов нет. Влияние на физику не проверял |
| execute | ExecuteCommand | ЧАСТИЧНО | `commands/execute.rs`. Ветки as/at/positioned/rotated/facing/align/anchored/in/summon/store/if работают. `if predicate` всегда ложь (`execute.rs:317`). `in` знает только три измерения. `if loaded` и `biome` есть |
| bossbar | BossBarCommands | ЕСТЬ | `commands/bossbar.rs`. Состояние в `g.cs.bossbar`, игроки по UUID |
| clear | ClearInventoryCommands | ЧАСТИЧНО | `commands/items.rs`. Без компонентов: предикат с компонентом всегда не совпадает |
| clone | CloneCommands | ЕСТЬ | `commands/blocks.rs`. Ветки и аргументы есть в дереве и исполнителе. Режимы masked и filtered и копирование block entity построчно не сверял (не уверен) |
| compute | ComputeCommand | ЧАСТИЧНО | `commands/misc/compute.rs`. Только константы (`1.5`, `3`). Именованные провайдеры и формы `block`/`entity` дают stub |
| damage | DamageCommand | ЧАСТИЧНО | `commands/entities.rs` (`do_damage`). Урон идёт через `interact::damage`. Только игроки. Аргументы `location` и `entity` (ветки `at location`, `by entity`) читаются и отбрасываются, на урон не влияют. Тип урона (`damageType`) учитывается |
| data | DataCommands | ЧАСТИЧНО | `commands/data.rs`. get/merge/modify/remove для блоков и хранилища полные. Для сущностей: игроки (запись отказана), предметы, TNT, падающие блоки. Остальные сущности нет |
| datapack | DataPackCommand | ЧАСТИЧНО | `commands/datapack.rs`. Список, включение и отключение, папки с `pack.mcmeta`. Что реально меняет игра из датапаков, не проверял (не уверен) |
| debug | DebugCommand | ЧАСТИЧНО | `commands/misc/profiling.rs`. `start` и `stop` работают. `function` всегда stub (трассировщика нет) |
| defaultgamemode | DefaultGameModeCommands | ЕСТЬ | `commands/gamemode.rs` (`default_mode`) |
| dialog | DialogCommand | ЧАСТИЧНО | `commands/misc/dialog.rs`. `show` по id и по inline-значению, `clear`. Есть ли диалоги в реестре Rine, не проверял (не уверен) |
| difficulty | DifficultyCommand | ЕСТЬ | `commands/world.rs` (`difficulty_frame`, пакет при входе). Эффектов на мобов нет, мобов нет |
| effect | EffectCommands | ЧАСТИЧНО | `commands/effect.rs`. Список, длительность, усилитель, частицы. Эффекты с серверным поведением только клиентские (см. сводку) |
| me | EmoteCommands | ЕСТЬ | `commands/chat.rs` |
| enchant | EnchantCommand | НЕТ | `commands/items.rs`. Зачарования не хранятся, команда не может успешно выполниться (`enchant_unmodelled`) |
| experience | ExperienceCommand | ЕСТЬ | `commands/experience.rs`. XP в отдельной карте по eid, очищается при выходе |
| xp | алиас experience | ЕСТЬ | то же |
| fill | FillCommand | ЕСТЬ | `commands/blocks.rs`. Режимы replace, destroy, keep, outline, hollow, все фильтры |
| fillbiome | FillBiomeCommand | ЕСТЬ | `commands/blocks.rs:486`. Меняет биомы в секциях, чанк шлётся целиком (ваниль шлёт только биомы) |
| forceload | ForceLoadCommand | ЕСТЬ | `commands/world.rs` (`forced`), `game/mod.rs` держит такие чанки при выгрузке |
| function | FunctionCommand | ЕСТЬ | `commands/function.rs`, `exec.rs`. Макросы, аргументы, очередь как в ванили |
| gamemode | GameModeCommand | ЕСТЬ | `commands/gamemode.rs`. Сообщения и смена режима |
| gamerule | GameRuleCommand | ЕСТЬ | `commands/gamerule.rs`, `rules.rs`. 58 правил, имена совпадают с деревом. Хранятся в `gamerules.txt`. Что каждое правило меняет в игре, проверил не все |
| give | GiveCommand | ЧАСТИЧНО | `commands/items.rs`. Предмет, количество. Компоненты не поддерживаются (стак хранит только id, count, damage) |
| help | HelpCommand | ЕСТЬ | `commands/players/help.rs` (как `getSmartUsage`) |
| item | ItemCommands | ЧАСТИЧНО | `commands/itemcmd.rs`. Слоты игрока, инвентарь, часть контейнеров. Компоненты, функции с data-packs, часть лута не поддерживаются (`unmodelled`) |
| kick | KickCommand | ЕСТЬ | `commands/players.rs` |
| kill | KillCommand | ЧАСТИЧНО | `commands/entities.rs` (`kill_one`). Игроки, предметы, TNT, падающие блоки. Мобов нет |
| list | ListPlayersCommand | ЕСТЬ | `commands/players.rs` |
| locate | LocateCommand | ЧАСТИЧНО | `commands/misc/locate.rs`. `biome` ищет по загруженным чанкам в радиусе 6400. `structure` и `poi` дают stub |
| loot | LootCommand | ЧАСТИЧНО | `commands/itemcmd.rs`. Только блочные таблицы. Рыбалка и остальные таблицы дают `unmodelled` |
| msg | MsgCommand | ЕСТЬ | `commands/chat.rs` |
| tell | алиас msg | ЕСТЬ | то же |
| w | алиас msg | ЕСТЬ | то же |
| swing | SwingCommand | ЧАСТИЧНО | `commands/entities.rs`. Только игроки |
| particle | ParticleCommand | ЧАСТИЧНО | `commands/sound.rs`. Все 7 форм разбираются. Список имён частиц написан вручную (`ParticleTypes` не портирован). Опции для всех типов не проверял |
| place | PlaceCommand | НЕТ | `commands/misc/place.rs`. Проверки аргументов, потом ошибки `feature.failed`, `structure.failed`, `template.invalid`, `jigsaw.failed`. Генерации структур и шаблонов нет |
| playsound | PlaySoundCommand | ЕСТЬ | `commands/sound.rs` |
| posteffect | PostEffectCommand | ЕСТЬ | `commands/misc/postfx.rs` |
| random | RandomCommand | ЕСТЬ | `commands/random.rs`. Xoroshiro128++, MD5 для id последовательности, как в ванили |
| reload | ReloadCommand | ЧАСТИЧНО | `commands/datapack.rs`. Перечитывает то, что Rine умеет (не уверен, что именно) |
| recipe | RecipeCommand | НЕТ | `commands/misc/advancement.rs`. `give` и `take` не находят рецепт, `*` ничего не меняет. Книги рецептов клиента нет |
| fetchprofile | FetchProfileCommand | ЧАСТИЧНО | `commands/misc/fetch.rs`. Поиск по имени и UUID всегда ошибка (нет API Mojang). Онлайн-игрок печатается без скинов |
| return | ReturnCommand | ЕСТЬ | `commands/function.rs` |
| ride | RideCommand | НЕТ | `commands/entities.rs` (`mount`). Проверки есть, посадки нет: транспорта нет, команда всегда отвечает «needs vehicles» |
| rotate | RotateCommand | ЕСТЬ | `commands/teleport.rs` |
| say | SayCommand | ЕСТЬ | `commands/chat.rs` |
| schedule | ScheduleCommand | ЕСТЬ | `commands/function.rs` (`schedule function`, `schedule clear`) |
| scoreboard | ScoreboardCommand | ЕСТЬ | `commands/scoreboard.rs`. Цели, очки, отображение, сохранение в `data/scoreboard.dat` |
| seed | SeedCommand | ЕСТЬ | `commands/players.rs` |
| setblock | SetBlockCommand | ЕСТЬ | `commands/blocks.rs` (`setblock`, режимы replace, destroy, keep, strict) |
| setidletimeout | SetPlayerIdleTimeoutCommand | ЧАСТИЧНО | `commands/players.rs:849`. Значение хранится. Кик по простою не проверял (не уверен) |
| spawnpoint | SetSpawnCommand | ЕСТЬ | `commands/world.rs` |
| setworldspawn | SetWorldSpawnCommand | ЕСТЬ | `commands/world.rs` |
| spectate | SpectateCommand | ЧАСТИЧНО | `commands/teleport.rs:390`. Проверки и переход есть. Камера и режим наблюдения не проверял (не уверен) |
| spreadplayers | SpreadPlayersCommand | ЕСТЬ | `commands/teleport.rs:152` |
| stopsound | StopSoundCommand | ЕСТЬ | `commands/sound.rs` |
| stopwatch | StopwatchCommand | ЧАСТИЧНО | `commands/stopwatch.rs`. Только в памяти, в `stopwatches.dat` не сохраняется |
| summon | SummonCommand | ЧАСТИЧНО | `commands/summon.rs`. Только `minecraft:item`, `minecraft:tnt`, `minecraft:falling_block`. Остальные типы: `commands.summon.failed` |
| tag | TagCommand | ЕСТЬ | `commands/entities.rs` (add, remove, list) |
| team | TeamCommand | ЕСТЬ | `commands/scoreboard.rs` |
| teammsg | TeamMsgCommand | ЕСТЬ | `commands/scoreboard.rs` |
| tm | алиас teammsg | ЕСТЬ | то же |
| teleport | TeleportCommand | ЕСТЬ | `commands/teleport.rs`. Все формы, включая facing и rotation |
| tp | алиас teleport | ЕСТЬ | то же |
| tellraw | TellRawCommand | ЕСТЬ | `commands/chat.rs` |
| test | TestCommand | НЕТ | `commands/misc/gametest.rs`. Набор тестов всегда пуст, `run` говорит «нет тестов», `create` stub. GameTest не портирован |
| tick | TickCommand | ЕСТЬ | `commands/world.rs` (`tick_should_run`, `tick_interval`). Вызывается из `game/mod.rs` |
| time | TimeCommand | ЕСТЬ | `commands/world.rs`. set, add, query, маркеры дня. Пересчёт маркеров приближённый (`world.rs:572`) |
| title | TitleCommand | ЕСТЬ | `commands/chat.rs` |
| trigger | TriggerCommand | ЕСТЬ | `commands/scoreboard.rs` |
| waypoint | WaypointCommand | ЧАСТИЧНО | `commands/misc/waypoint.rs`. Передатчики только игроки (мобов нет). Цвет и стиль хранятся, `stub` для остальных ветвей |
| weather | WeatherCommand | ЧАСТИЧНО | `commands/world.rs`. Состояние и пакеты есть. Эффектов погоды (дождь, молнии, тушение огня) в Rine не нашёл |
| worldborder | WorldBorderCommand | ЧАСТИЧНО | `commands/world.rs` (`Border`, lerp, пакеты). Урона за границей и отталкивания игрока не нашёл |
| jfr | JfrCommand | НЕТ | `commands/misc/profiling.rs`. `start` всегда ошибка, `stop` даёт «Not currently profiling» |
| ban-ip | BanIpCommands | ЕСТЬ | `commands/players.rs`, списки в JSON как в ванили (`players/files.rs`) |
| banlist | BanListCommands | ЕСТЬ | `commands/players.rs` |
| ban | BanPlayerCommands | ЕСТЬ | `commands/players.rs` |
| deop | DeOpCommands | ЕСТЬ | `commands/players.rs` |
| op | OpCommand | ЕСТЬ | `commands/players.rs` |
| pardon | PardonCommand | ЕСТЬ | `commands/players.rs` |
| pardon-ip | PardonIpCommand | ЕСТЬ | `commands/players.rs` |
| perf | PerfCommand | ЧАСТИЧНО | `commands/misc/profiling.rs`. Таймер TPS. Файл отчёта не пишется |
| save-all | SaveAllCommand | ЕСТЬ | `commands/players.rs` (с `flush` тоже) |
| save-off | SaveOffCommand | ЕСТЬ | `commands/players.rs`. Влияет ли флаг на автосохранение, не проверял (не уверен) |
| save-on | SaveOnCommand | ЕСТЬ | `commands/players.rs` |
| stop | StopCommand | ЕСТЬ | `commands/players.rs:807` |
| transfer | TransferCommand | ЕСТЬ | `commands/players.rs:439`. Пакет `minecraft:transfer` в коде есть, в живом клиенте не проверял |
| version | VersionCommand | ЕСТЬ | `commands/players.rs` |
| whitelist | WhitelistCommand | ЕСТЬ | `commands/players.rs`. Проверка при входе есть (`players.rs:331`, `game/mod.rs:480`) |

Команды из `V/server/commands`, которых нет в дереве релизного сервера:

| Ванильный класс | Статус | Почему |
|---|---|---|
| ChaseCommand | N/A | регистрируется только при `DEBUG_CHASE_COMMAND` |
| RaidCommand, DebugPathCommand, DebugMobSpawningCommand, WardenSpawnTrackerCommand, SpawnArmorTrimsCommand, ServerPackCommand, DebugConfigCommand | N/A | только при `DEBUG_DEV_COMMANDS` или в IDE |
| PublishCommand, UnpublishCommand | N/A | только в integrated-сервере |
| EmoteCommands | ЕСТЬ | это команда `me` (см. выше) |

Дополнительно: `V/server/commands/item/` (аксессоры `/item` и `/loot`) и `V/server/commands/data/` (аксессоры `/data`) покрыты через `itemcmd.rs` и `data.rs`, см. строки выше. `V/server/commands/CommandResponseTracker.java` есть как `commands/misc/tracker.rs` (ЕСТЬ). `ArgProvider.java`, `LookAt.java`, `InCommandFunction.java`: куда именно они легли в Rine, не проверял (не уверен), функции `ExecuteCommand` и `function` покрыты в `exec.rs` и `function.rs`.

## Аргументы и селекторы

Парсеры: `V/commands/arguments/` (47 классов) и `V/commands/arguments/{blocks,coordinates,item,selector}/`.

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| arguments/* (62 парсера из `command_argument_type`) | Разбор значений | ЕСТЬ | `crates/rine-command/src/args/mod.rs` (`from_parser`, `parse`). Все 62 варианта разбираются. Ошибки построены по `Reader` (копия `StringReader`) |
| arguments/selector/EntityArgument, ScoreHolderArgument, EntitySelectorParser | Селекторы @a/@p/@e/@r/@s, опции | ЧАСТИЧНО | `rine-command/src/args/selector.rs` (разбор всех опций: distance, level, x/y/z, dx/dy/dz, rotations, limit, sort, gamemode, team, type, tag, nbt, scores, advancements, predicate). Вычисление в `rine-server/src/game/cmd/select.rs`. `advancements=` никогда не совпадает (`select.rs:170`). При `advancements={x=false}` ни один игрок не подойдёт, в ванили подошли бы все без этого достижения. `predicate=` без отрицания никогда не совпадает (`select.rs:171`) |
| arguments/blocks/BlockStateArgument, BlockPredicateArgument | Состояние и предикат блока | ЕСТЬ | `rine-command/src/args/block.rs`, `execute.rs::block_test`. Теги блоков через `registries.tag_contains` |
| arguments/item/ItemArgument, ItemPredicateArgument, SlotArgument, SlotsArgument, SlotSourceArgument | Предметы и слоты | ЧАСТИЧНО | `rine-command/src/args/item.rs`, `commands/items.rs`. Разбор есть. Предикаты с компонентами всегда не совпадают (`items.rs:161`) |
| arguments/ComponentArgument, StyleArgument, MessageArgument | Текст-компоненты, стили | ЧАСТИЧНО | `args/component.rs`, `cmd/select.rs::resolve_text`. Разбор и подстановка селекторов есть. Какие события клика и наведения клиент покажет, не проверял (не уверен) |
| arguments/CompoundTagArgument, NbtTagArgument, NbtPathArgument | SNBT, пути NBT | ЕСТЬ | `args/snbt.rs` (1795 строк), `args/nbt_path.rs`, `commands/data.rs` |
| arguments/ObjectiveArgument, ObjectiveCriteriaArgument, TeamArgument, TeamColorArgument, OperationArgument, ScoreboardSlotArgument | Скоринборд | ЕСТЬ | `args/misc.rs`, `commands/scoreboard.rs` |
| arguments/ParticleArgument | Частица с опциями | ЧАСТИЧНО | `args/misc.rs::parse_particle`, `commands/sound.rs`. Список имён вручную |
| arguments/HeightmapTypeArgument, EntityAnchorArgument, GameModeArgument, SwingAnimationArgument, TemplateMirrorArgument, TemplateRotationArgument | Перечисления | ЕСТЬ | `args/misc.rs` |
| arguments/DimensionArgument | Измерение | ЧАСТИЧНО | Парсится как id. В `execute.rs::dimension` принимаются три имени, миров Незер и Энд нет |
| arguments/GameProfileArgument | Профиль игрока | ЧАСТИЧНО | `args/selector.rs::parse_profile`. Поиск по имени только среди онлайн-игроков |
| arguments/ResourceArgument, ResourceKeyArgument, ResourceOrIdArgument, ResourceOrTagArgument, ResourceOrTagKeyArgument, ResourceSelectorArgument, IdentifierArgument | Ресурсы и теги реестров | ЧАСТИЧНО | `args/misc.rs`. Разбор есть. Значения только из реестров, которые Rine загружает из jar (`rine-data`) |
| arguments/RangeArgument (int/float range) | Диапазоны | ЕСТЬ | `args/misc.rs` (`IntRange`, `FloatRange`) |
| arguments/UuidArgument | UUID | ЕСТЬ | `args/misc.rs::parse_uuid` |
| arguments/AngleArgument, coordinates/* | Углы, координаты, `~` и `^` | ЕСТЬ | `args/coords.rs` (Mth.atan2 таблицей, как в ванили) |
| arguments/SignedArgument, ArgumentSignatures, StringRepresentableArgument | Подписи чата | НЕТ | Подписанные сообщения не нужны при offline-режиме (`online_mode = false`). Не уверен, что Rine такие пакеты принимает |
| arguments/TimeArgument | Время | ЕСТЬ | `args/misc.rs::parse_time` |
| arguments/NbtPathArgument | Пути NBT | ЕСТЬ | то же |
| arguments/ContextProvider/loot providers (`util/valueproviders`) | Числовые провайдеры для `compute` и лута | ЧАСТИЧНО | `args/misc.rs::parse_provider`. Исполнение только для констант (`compute` и числа в `commands/itemcmd.rs`) |
| arguments/LootTable, LootPredicate, LootModifier, Dialog, Feature | Ссылки на данные | ЧАСТИЧНО | Парсятся как id или inline. Исполнение: лут только блочный, predicate не выполняется, dialog и feature зависят от реестров |

## Остальные группы

Мои группы по `V/` (кроме команд и аргументов выше). Колонка «Где в Rine» указывает модуль или файл Rine.

### commands/ (верхний уровень, execution, functions, synchronization)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| commands/Commands.java | Диспетчер, регистрация, пакет дерева команд | ЕСТЬ | `rine-command/src/dispatch.rs`, `tree.rs`, `packet.rs` (пакет дерева с флагами и полем `suggest`), `rine-server/src/game/cmd/mod.rs` (`Commands`). Список команд см. выше |
| commands/CommandSourceStack, CommandSource, ExecutionCommandSource | Источник команды: позиция, права, куда выводить | ЕСТЬ | `rine-server/src/game/cmd/source.rs` (`Source`, `Output`), `feedback.rs` |
| commands/CommandSigningContext | Подписи команд и чата | НЕТ | Подписей нет, сервер offline |
| commands/SharedSuggestionProvider | Подсказки и списки имён | ЧАСТИЧНО | Общий класс подсказок на сервере не портирован, см. строку synchronization |
| commands/synchronization/SuggestionProviders, ArgumentTypeInfos, ArgumentTypeInfo, SingletonArgumentInfo, ArgumentUtils, brigadier/* | Подсказки аргументов по запросу клиента (`ask_server`) | НЕТ | В `packet.rs:118` поле `suggest` (например `minecraft:ask_server`, `tree.rs:170`) уходит клиенту в дереве. Но входящий пакет `S_COMMAND_SUGGESTION` (id 15, `rine-protocol/src/ids.rs:136`) в `rine-server` нигде не обрабатывается, ответа `C_COMMAND_SUGGESTIONS` нет. Итог: Tab не подсказывает значения, которые клиент просит у сервера. Какие подсказки клиент считает сам, не проверял (не уверен) |
| commands/execution/* (ExecutionContext, Frame, ChainModifiers, CommandQueueEntry, tasks/*, TraceCallbacks и др.) | Очередь исполнения, цепочки, форки | ЕСТЬ | `rine-server/src/game/cmd/exec.rs` (порт `ExecutionContext`, лимиты цепочки и форков, `Callback`, `Frame`) |
| commands/functions/* (CommandFunction, MacroFunction, PlainTextFunction, StringTemplate, FunctionBuilder, InstantiatedFunction) | Функции и макросы | ЕСТЬ | `rine-server/src/game/cmd/commands/function.rs` (включая `StringTemplate`, `MacroFunction`) |
| commands/CommandBuildContext, CommandResultCallback, BrigadierExceptions | Контекст сборки, результат, стандартные ошибки | ЕСТЬ | `rine-command/src/error.rs` (`builtin`, `CmdError`), `rine-server/src/game/cmd/exec.rs` |
| commands/ParserUtils, commands/ArgumentVisitor | Служебные обходы | НЕ УВЕРЕН | Соответствий не искал, в обзоре не важно |

### core/ (123 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| core/BlockPos, Vec3i, Position, SectionPos, QuartPos, GlobalPos, PositionAndRotation, Rotations, FrontAndTop, Direction, Directional, AxisCycle, CompositeDirection, BlockMath, Cursor3D, BlockBox, Cloner | Координаты, направления, коробки | ЕСТЬ | `rine_protocol::BlockPos`, `rine-server/src/game/blockinv.rs` (`dir_of`, `dir_index`), `place.rs`, `commands/blocks.rs` (коробки и клонирование). Направления разобраны по месту, отдельного класса нет |
| core/UUIDUtil, Position | Утилиты | ЕСТЬ | `uuid` крейт |
| core/registries/* (Registry, MappedRegistry, RegistryAccess, HolderLookup, HolderSet, Holder, RegistryCodecs, RegistrySynchronization, LayeredRegistryAccess, RegistrySetBuilder) | Реестры и теги | ЕСТЬ | `crates/rine-data/src/registry.rs` (load, id, name, tag, tag_contains, synced, tags_payload). Реестры и теги берутся из jar 26.3 (`registries.json`, `tags.json`). Значения из датапаков не загружаются (не уверен, насколько это важно сейчас) |
| core/component/* (DataComponent*, DataComponentPatch, PatchedDataComponentMap, TypedDataComponent, Removed) | Компоненты предметов | НЕТ | `game/player.rs:24` (`Stack`: id, count, damage). Только `minecraft:damage`. Остальное нет |
| core/component/predicates/* (~18 файлов) | Предикаты компонентов | НЕТ | Нет. См. `items.rs:161` |
| core/dispenser/* (11 файлов) | Поведение раздатчика | ЧАСТИЧНО | `game/blockinv.rs::dispense`, `drop_one`. Есть только `DefaultDispenseItemBehavior` и вариант для выбрасывателя. Лодки, вагонетки, броня (`EquipmentDispenseItemBehavior`), огниво, ножницы, яйца призыва, снаряды (`ProjectileDispenseBehavior`), шалкер-боксы, `OptionalDispenseItemBehavior` не реализованы |
| core/cauldron/* | Взаимодействие с котлами | НЕТ | В Rine есть только состояние котла в редстоуне (`redstone/engine.rs:895`, `data.rs:843`). Взаимодействия ведром и бутылкой нет |
| core/particles/* (~20 файлов) | Опции частиц | ЧАСТИЧНО | `commands/sound.rs` (имена частиц вручную), `misc.rs::parse_particle` (разбор). Полная таблица `ParticleTypes` не портирована |
| core/ClientAsset, IdMap, IdMapper, DefaultedRegistry, DefaultedMappedRegistry, WritableRegistry, PatchedRegistry, ConcurrentHolderGetter, TypedInstance, RegistrationInfo | Служебные классы реестров | ЕСТЬ | Сводно: `rine-data/src/registry.rs`. Отдельных аналогов не нужно |
| core/RegistryAccess, Registries, BuiltInRegistries, BootstrapRegistry, MultiRegistryBootstrap, SingleRegistryBootstrap, RegistryFileCodec, RegistryFixedCodec | Загрузка реестров | НЕ НУЖНО | Инициализация из jar через `rine-data`. Кодеки и bootstrap не портированы, в рантайме не нужны |

### advancements/ (115 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| advancements/Advancement, AdvancementHolder, AdvancementNode, AdvancementTree, AdvancementProgress, CriterionProgress, AdvancementRequirements, AdvancementRewards, DisplayInfo, AdvancementType, TreeNodePosition, CriterionTriggerInstance | Дерево, прогресс, награды | НЕТ | Нет. Rine не грузит дерево достижений (`commands/misc/advancement.rs`, шапка). Пакеты `update_advancements` и `select_advancements_tab` не отправляются |
| advancements/triggers/* (~65 файлов, CriteriaTriggers, Criterion, SimpleCriterionTrigger, PlayerTrigger и триггеры вроде InventoryChange, PlayerHurtEntity, KilledTrigger, ChangeDimension) | Критерии и триггеры | НЕТ | Нет. Ни один триггер не срабатывает, прогресс не ведётся |
| advancements/predicates/* (~50 файлов) | Предикаты для критериев и селекторов | НЕТ | Нет. Частично есть разбор `predicate=` и `advancements=` в селекторе, но проверки нет (`select.rs:170`) |
| advancements/predicates/entity/* (~30 файлов) | Предикаты сущностей | НЕТ | Нет. Для предикатов сущностей нужны мобы и тип сущности, их в Rine нет |

### nbt/ (43 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| nbt/Tag, ByteTag, ShortTag, IntTag, LongTag, FloatTag, DoubleTag, StringTag, ListTag, CompoundTag, ByteArrayTag, IntArrayTag, LongArrayTag, EndTag, NumericTag, PrimitiveTag, CollectionTag, TagType, TagTypes | Типы NBT | ЕСТЬ | `crates/rine-protocol/src/nbt.rs` (enum `Tag` с 12 видами) |
| nbt/NbtIo, NbtUtils, NbtOps, SnbtOperations, SnbtGrammar, TagParser, SnbtPrinterTagVisitor, StringTagVisitor, TextComponentTagVisitor, TagVisitor, StreamTagVisitor | Чтение, запись, SNBT | ЕСТЬ | `rine-protocol/src/nbt.rs` (бинарный формат), gzip проверен на `scoreboard.dat` (`scoreboard.rs`), `rine-command/src/args/snbt.rs` (разбор и печать) |
| nbt/NbtAccounter, NbtAccounterException, ReportedNbtException | Лимиты размера и глубины | НЕ УВЕРЕН | Не нашёл аналога лимитов в `rine-protocol/src/nbt.rs`. Не проверял, есть ли защита от больших NBT в сетевом разборе |
| nbt/visitors/* (CollectFields, CollectToTag, FieldSelector, FieldTree, SkipAll, SkipFields) | Выборка полей при разборе | НЕТ | Нет аналога, Rine читает целиком. Для команд обычно не нужно |

### tags/ (~30 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| tags/TagKey, TagLoader, TagFile, TagEntry, TagBuilder, TagNetworkSerialization, BlockTags, ItemTags, BlockItemTags, FluidTags, EntityTypeTags, BiomeTags, DamageTypeTags, ... | Теги | ЧАСТИЧНО | `rine-data/src/registry.rs` (`tag`, `tag_contains`, `tags_payload`). Теги из jar ванильного ядра. Загрузка тегов из датапаков не нашёл (не уверен) |

### stats/ (13 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| stats/ServerStatsCounter, StatsCounter, Stat, StatType, Stats, StatFormatter, RecipeBook* | Счётчики статистики | НЕТ | Нет. Пакета `award_stats` нет, файлов статистики нет. Для скоринборда это важно: критерии `deathCount`, `playerKillCount`, `totalKillCount` и `minecraft.*:*` принимаются командой (`rine-command/src/args/misc.rs`, `CUSTOM_CRITERIA` и `criteria_name`), но никогда не растут. В игровом коде нет записи в скоринборд, кроме команд (проверил grep по `cs.scoreboard` и `commands::scoreboard::`). Критерии `health`, `food`, `air`, `armor`, `xp`, `level` тоже только читаются, игра их не обновляет (`commands/scoreboard.rs:225`, `criteria_of`) |

### recipebook/ (2 файла) и крафт

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| recipebook/ServerPlaceRecipe, PlaceRecipeHelper | Раскладка рецепта в книгу и сетку | НЕТ | Нет. Крафт есть (`game/containers.rs`, рецепты из `rine-data/assets/recipes.json`), но без книги рецептов клиента, пакетов `recipe_book_*` и без разблокировки |
| stats/RecipeBook, RecipeBookSettings, ServerRecipeBook | Состояние книги | НЕТ | Нет |

### sounds/ (3 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| sounds/SoundEvent, SoundEvents, SoundSource, Music, Musics | События и категории звуков | ЧАСТИЧНО | `commands/sound.rs` (`CATEGORIES`, список категорий вручную), `game/` (звуки копания и установки блоков по ванильной схеме). Таблица `SoundEvents` не портирована, пути звуков взяты по месту |

### resources/ (~15 файлов) и locale/ (3 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| resources/Identifier, ResourceKey, DependantName, FileToIdConverter, IdentifierPattern (`util/IdentifierPattern`) | Идентификаторы и ключи | ЕСТЬ | `rine-command/src/args/misc.rs::read_id`, `rine-command/src/error.rs`. Строки `namespace:path` |
| resources/RegistryDataLoader, RegistryLoadTask, ResourceManagerRegistryLoadTask, NetworkRegistryLoadTask, RegistryOps, RegistryValidator, DelegatingOps | Загрузка данных реестров | ЧАСТИЧНО | Данные только из jar (`rine-data`). Датапак-загрузки нет |
| locale/Language, DeprecatedTranslationsInfo | Переводы | ЧАСТИЧНО | `rine-command/src/lang.rs` и `assets/en_us.json` из jar. Только английский. Нужен для консоли и `getString()` |

### references/ (4 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| references/BlockIds, BlockItemId, BlockItemIds, ItemIds | Константы id | ЕСТЬ | `rine-data/src/blocks.rs`, `items.rs`, `assets/blocks.json`, `items.json`. Id как строки |

### data/ (174 файла)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| data/* (генераторы: advancements, loot, recipes, registries, structures, tags, worldgen, metadata, info) | Генерация JSON для датапаков | НЕ НУЖНО | Серверу в рантайме не нужно. Нужные данные уже вынуты из jar в `rine-data/assets` (`block_loot.json`, `recipes.json`, `registries.json`, `tags.json`, `datapack_registries.json`) |

### gametest/ (~45 файлов) и gizmos/ (~13 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| gametest/framework/* (GameTestHelper, GameTestRunner, TestCommand и др.) | Фреймворк тестов | НЕТ | `commands/misc/gametest.rs` отвечает пустым набором. Тестов и тестовых блоков нет |
| gizmos/* | Отладочная отрисовка на клиенте | НЕТ | Не нужно серверу, пакетов нет |

### util/ (735 файлов) и корень `minecraft/`

Инфраструктуру и почти весь util не разбирал, по требованию. Ниже только то, что влияет на геймплей или совместимость.

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| util/RandomSource, util/random/*, util/LinearCongruentialGenerator | Генератор случайных чисел | ЕСТЬ | `commands/random.rs` (Xoroshiro128++), `rine-worldgen` (шумы и рандом, сверено с jar). Остальное не смотрел |
| util/Mth | Математика | ЧАСТИЧНО | Нужное для генерации есть в `rine-worldgen`. Полной копии `Mth` нет, в геймплее отдельно не сверялось |
| util/CubicSpline, util/Keyframe, util/KeyframeTrack*, util/Ease, util/EasingType, util/BinaryAnimator, util/SmoothDouble | Сплайны и анимации | НЕ УВЕРЕН | Сплайны могут нужны генератору (`rine-worldgen`), не проверял. Анимации клиентские, не нужны |
| util/ExtraCodecs, util/HashOps, util/NullOps, util/Unit, util/DelegateDataOutput, util/StrictJsonParser, util/LenientJsonParser, util/GsonHelper | Кодеки и JSON | НЕТ | Rine разбирает JSON через `serde_json`, кодеков Mojang нет. В рантайме не требуется, но в ванильных данных может быть формат, который Rine не читает (не проверял) |
| util/valueproviders/* (19 файлов) | Числовые провайдеры | ЧАСТИЧНО | См. таблицу аргументов. Исполнение только для констант |
| util/profiling/* (70 файлов), util/debug*, util/monitoring/*, util/eventlog | Профайлер, отладка | НЕТ | Только `/perf` и `/debug start|stop` на таймере (`commands/misc/profiling.rs`) |
| util/datafix/* (411 файлов), util/filefix/* (57 файлов) | Миграция старых форматов | НЕ УВЕРЕН | Rine пишет свой формат с `DataVersion` 26.3. Миграции старых миров (`DataFixer`) не портированы. Не проверял, что Rine делает со старым `level.dat` другой версии |
| util/parsing/* (29 файлов) | Разбор данных | НЕТ | Rine использует свой `Reader` (`rine-command/src/reader.rs`) |
| util/Util, util/thread/*, util/context/*, util/worldupdate/*, util/TickThrottler, util/TaskChainer, util/Graph, util/DependencySorter, util/ModCheck, util/Continuation | Инфраструктура | НЕ НУЖНО | Rine: свой цикл и потоки (`game/mod.rs`, `rine-world`), на этом не завязан |
| util/Crypt, util/Signer, util/Signature*, util/SignatureValidator, util/PngInfo, util/HttpUtil, util/FileZipper | Подписи, криптография, HTTP, PNG | НЕТ | Offline-режим (`online_mode = false`, `README.md`). Подписи чата и Mojang API не нужны |

### Корень `minecraft/` (19 файлов)

| Ванильный файл(ы) | Механика | Статус | Где в Rine / что не хватает |
|---|---|---|---|
| ChatFormatting | Цвета и стили текста | ЕСТЬ | `rine-protocol/src/text.rs` (`Text::color`, `bold`, `italic`, `underlined`) |
| SharedConstants, WorldVersion, DetectedVersion | Версия протокола и данных | ЕСТЬ | `rine-protocol/src/ids.rs` (протокол 777), `DATA_VERSION` в `scoreboard.rs` и других файлах |
| CharPredicate, Optionull, IdentifierException, ExitCodes, ReportType, ReportedException, SuppressForbidden, DefaultUncaughtExceptionHandler*, TracingExecutor | Служебные классы | НЕ НУЖНО | Rine собран с `panic = "abort"` (`docs/COMMANDS-PORT.md`), своих отчётов о падении нет |
| CrashReport, CrashReportCategory, CrashReportDetail, SystemReport | Отчёты о падении | НЕ НУЖНО | Нет аналога, не требуется |

## Важнее всего, чего нет

1. Достижения целиком: дерево, триггеры, прогресс, пакеты `update_advancements`, `/advancement`, фильтр `advancements=` в селекторе. Это самая большая дыра в этой части.
2. Компоненты предметов и зачарования: `/enchant` не может успешно выполниться, `/give` с компонентами и предикаты компонентов не работают. Без этого не будет нормального инвентаря и лута.
3. Серверная механика эффектов и атрибутов: скорость, сила, регенерация, урон от яда и т.п. не влияют на игру (по комментарию в `effect.rs`). Проверить в живой игре не смог, только по коду.
4. Мобы: почти все «entity»-команды работают только с игроками, предметами, TNT и падающими блоками. Это ограничивает `kill`, `damage`, `swing`, `ride`, `summon`, `data entity`, `attribute`, `waypoint`.
5. Структуры и шаблоны: `/place structure|template|feature|jigsaw` и `/locate structure|poi` заглушки. Конфигурированные фичи из `rine-worldgen` не подключены к команде.
6. Статистика и книга рецептов клиента: `stats/` и `recipebook/` не портированы. Из-за этого пропадает счётчик статистики и книга крафта.
7. Диспенсер: только базовое поведение. Лодки, вагонетки, броня, огниво, ножницы, яйца призыва, снаряды не работают.
8. Погода, граница мира и измерения: состояние есть, эффектов в игре не нашёл (погода, урон за границей). Незер и Энд отсутствуют.
9. Предикаты (`execute if predicate`, селекторы `predicate=`, `advancements=` с `false`) и лут не-блочный: всегда ложь или ошибка.
10. Датапаки: список и переключение есть, но данные из папок датапаков в реестры и теги, судя по коду, не попадают (не уверен, проверил не до конца).
11. Tab-подсказки: `S_COMMAND_SUGGESTION` не обрабатывается. Игроки не видят подсказок по id, функциям, звукам и т.п. Это видно сразу, поэтому в списке высоко.
12. Скоринборд от игры: критерии смертей, убийств и статистики принимаются, но никто их не увеличивает. Критерии здоровья и еды только читаются.

## Что не смотрел или не уверен

- Поведение каждой ветки команд построчно сверено не со всеми 92 командами, статусы по шапкам модулей и по заглушкам. Для ЕСТЬ-статусов оговорки в колонке «что не хватает» есть там, где я их увидел.
- Клиентское отображение (клики, наведение, звуки, частицы в игре) не проверялось: нет живого клиента.
- `util/datafix`, `util/profiling`, сплайны генератора и лимиты NBT: не уверен, насколько они важны.
- Тестов не запускал, кроме `registry_paths_valid`. Сборка не менялась, код не трогал.
