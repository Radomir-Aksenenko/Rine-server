# Перенос ванильных команд: руководство для исполнителей

Цель: каждая команда со стороны игрока выглядит и ведёт себя как в ванильном
26.3 (те же ошибки, те же сообщения, те же коды результата), а внутри
работает быстро. Источник истины: исходники
`Paper-26.3/paper-server/src/minecraft/java/net/minecraft/`. Всё, что помечено
`// Paper` / `// CraftBukkit` / `// Spigot`, это не ваниль: берите ванильное
поведение (если неясно, какое оно, опишите это в отчёте).

## Где что лежит

- `crates/rine-command` (не зависит от игры):
  - `assets/tree.json`: дерево команд, снятое с ванильного сервера на уровнях
    прав 0-4. Узел знает путь (`"gamemode gamemode target"`), тип аргумента,
    уровень. Грамматику руками не пишем.
  - `reader.rs`: `Reader`, копия Brigadier `StringReader` (те же ошибки).
    Курсор в байтах. `read_int/read_float/read_double/read_long/read_boolean`,
    `read_string` (кавычки), `read_unquoted_string`, `read_quoted_string`,
    `expect(c)`, `peek()`, `skip()`, `can_read()`, `can_read_n(n)`,
    `skip_whitespace()`, `remaining()`, `cursor` (pub).
    `Reader::is_allowed_in_unquoted_string(c)` = `[0-9A-Za-z_.+-]`.
  - `error.rs`: `CmdError { msg: Text, input, cursor }`.
    `CmdError::at(Text, &reader)` = Brigadier `createWithContext(reader)`,
    `CmdError::new(Text)` / `CmdError::key("k")` / `CmdError::tr("k", args)` =
    `create()`. `builtin::*` = стандартные ошибки Brigadier.
    `lit(v)` делает аргумент перевода из числа/строки (`translatableEscape`).
    `java_double(f64)`, `java_float(f32)` печатают числа как Java.
  - `args/mod.rs`: `ArgType` (62 парсера) и `Arg` (значения). Значения не
    зависят от источника: селекторы и координаты разрешаются при исполнении.
  - `args/*.rs`: парсеры. Каждая функция-заглушка описана doc-комментарием.
  - `lang.rs`: английские строки (`lang::plain(&Text)` = `getString()`).
- `crates/rine-protocol/src/text.rs`: `Text` (как ванильный `Component`):
  `Text::plain(s)`, `Text::key("k")`, `Text::tr("k", vec![args])`,
  `.color("red")`, `.bold()`, `.italic()`, `.underlined()`, `.push(child)`,
  `.on_click(click_suggest(..))`, `.on_hover(hover_text(&t))`, `.insertion()`.
  Сообщения игроку всегда `Text::tr` с ванильным ключом, не готовая строка.
- `crates/rine-server/src/game/cmd/`:
  - `mod.rs`: `Commands` (реестр исполнителей), `Ctx` (контекст исполнителя),
    `perform(g, src, line)`, `report_parse_error`, `EntMeta`.
  - `exec.rs`: очередь исполнения (порт `ExecutionContext`), `Callback`,
    `Frame`, `Mods`, `Function`, `queue_command`, `queue_function`.
  - `source.rs`: `Source` (= `CommandSourceStack`), `Output`.
  - `feedback.rs`: `send_success`, `send_failure`, `send_system`,
    `display_name(g, eid)`, `player_display_name(g, slot)`,
    `entity_type_id(g, eid)`, `item_description_id(item)`.
  - `select.rs`: селекторы и разрешение текста: `find_entities`,
    `find_players`, `entity_pos`, `entity_rot`, `scoreboard_name`,
    `plain_name`, `text_from_tag`, `resolve_text`, `join_names`.
  - `rules.rs`: игровые правила, `g.rules.bool("send_command_feedback")`,
    `g.rules.int("max_block_modifications")`.
  - `commands/<имя>.rs`: исполнители, один модуль на ванильный класс.

## Как пишется исполнитель

```rust
pub fn register(c: &mut Commands) {
    // путь = имена узлов от корня через пробел, как в tree.json
    c.exec("gamemode gamemode", |x| { ... Ok(count) });
    c.exec("gamemode gamemode target", other_fn);
}

fn other_fn(x: &mut Ctx) -> CmdResult<i32> {
    let mode = x.enum_("gamemode");          // имя аргумента = имя узла
    let targets = x.players("target")?;      // ошибки селектора как в ванили
    x.success(Text::tr("commands.x.success", vec![x.player_name(slot)]), true);
    Err(CmdError::key("commands.x.failed"))  // ошибка: красное сообщение
}
```

- Возвращаемое число = результат команды (для `execute store`).
- `Err(CmdError)` = ванильный `throw ...Exception.create()`. Красный цвет и
  вывод делает очередь, сами не отправляйте.
- `x.success(text, broadcast)` = `source.sendSuccess(() -> text, broadcast)`.
- `register` модуля вызывается из `commands/mod.rs::register_all`.
- Пути проверяются при старте: опечатка в пути роняет запуск (это нарочно).
- Литерал на позиции i в текущей стадии: `x.lit(i)` (0 = имя команды).
- Наличие необязательного аргумента: `x.has("name")`.

### Геттеры `Ctx`

| Тип аргумента в tree.json | Геттер |
|---|---|
| brigadier:integer / long / float / double / bool | `x.int(n)`, `x.long(n)`, `x.float(n)`, `x.double(n)`, `x.bool(n)` |
| brigadier:string, objective, team, objective_criteria | `x.str(n)` |
| resource_location, dimension, resource, resource_key | `x.id(n)` (`namespace:path`) |
| gamemode, entity_anchor, heightmap, template_* , swing_animation | `x.enum_(n)` |
| entity | `x.entities(n)?` (eid, ошибка если пусто), `x.optional_entities(n)?`, `x.entity(n)?`, `x.players(n)?` (слоты), `x.optional_players(n)?`, `x.player(n)?` |
| game_profile | `x.profiles(n)?` → `Vec<(String, Uuid)>` |
| vec3 / vec2 / rotation | `x.vec3(n)` `[f64;3]`, `x.vec2(n)`, `x.rotation(n)` `[xRot, yRot]`, `x.coords(n)` (сырые, `.is_relative(axis)`) |
| block_pos | `x.block_pos(n)?` (загружен и в мире), `x.block_pos_unchecked(n)`, `x.spawnable_pos(n)?` |
| column_pos | `x.column_pos(n)` |
| component | `x.component(n)?` (разрешённый `Text`) |
| message | `x.message(n)?` |
| nbt_tag / nbt_compound_tag / style | `x.nbt(n)` (`&Tag`) |
| остальное | `x.arg(n)` и `match` по `Arg` |

Источник: `x.src` (`pos`, `rot`, `level`, `entity`, `text_name`, `display`,
`silent`). `x.source_player()?` = `getPlayerOrException`,
`x.source_entity()?` = `getEntityOrException`. `x.pose()` для координат.

### Состояние игры (`x.g: &mut Game`)

- Игроки: `g.players: Vec<Option<Player>>` по слотам, `g.player(slot)`,
  `g.find_player(name)`, `g.by_eid: eid → слот`. В `Player`: `name`, `uuid`,
  `eid`, `pos`, `yaw`, `pitch`, `gamemode`, `health`, `food`, `inv`,
  `p.msg(&text)`, `p.send(packet)`, `p.teleport(x, y, z, yaw, pitch)`.
- Сущности: `g.entities.get(eid)`, `g.entities.list` (`Ent`: `eid`, `uuid`,
  `kind` = Item/Tnt/Falling, `pos`, `vel`, `removed`). Спавн:
  `entity::spawn_item`, `spawn_tnt`, `spawn_falling`. Удалить: `removed = true`.
  Мобов пока нет.
- Метаданные для команд (теги, имя, ...): `g.ent_meta: HashMap<eid, EntMeta>`.
- Мир: `g.world.get_block(x, y, z) -> u16`,
  `g.world.set_block(BlockPos::new(x, y, z), state) -> old` (изменения уходят
  клиентам пачкой по секциям сами), `g.world.is_loaded(x, z)`.
  Блоки: `rine_data::get().blocks` (`parse_state`, `default_state`,
  `block_of(state).name`, `get_prop`, `with_prop`).
- Время: `g.day_time`, `g.tick`. Спавн: `g.spawn`.
- Пакеты: `crate::out::*` (`system_chat`, `game_event`, `sound`, ...).
  Нового пакета нет: формат берите из
  `net/minecraft/network/protocol/game/Clientbound*.java`, id из
  `rine_protocol::ids::play::C_*` (сверено с `vanilla-data/generated/reports/packets.json`).
- Широковещательно: `g.broadcast.push(out::frame(pkt))`.

## Правила

1. Портировать с исходников, не по памяти. Ключи переводов, порядок
   аргументов перевода, коды возврата и условия ошибок как в Java.
2. Сообщение `sendSuccess(..., true)` идёт админам через `x.success(t, true)`.
3. Не паниковать в исполнителях (сервер собран с `panic = "abort"`).
4. Горячие пути без лишних аллокаций и строковых сравнений в циклах; массовые
   изменения блоков через `g.world.set_block` (они пакуются по секциям).
5. Трогать только назначенные файлы. Новое общее состояние: в своём модуле,
   поле в `Game` добавить можно (одна строка в `game/mod.rs`), но сообщите.
6. Комментарии в коде на английском, коротко, без длинных тире.
7. Проверка: `cargo test -p rine-command` и `cargo check -p rine-server`
   (из корня). Порт 25565 занят, тестовый сервер только на других портах.
8. В конце: закоммитить в свою ветку (`git add <свои файлы>`, сообщение
   `feat(commands): ...` по-английски с
   `Co-Authored-By: Claude Haiku 5.5 <noreply@anthropic.com>`), коротко
   отчитаться: что сделано, что не удалось, что не проверено.
