# Аудит: сервер и сеть (ваниль V\server и V\network) против Rine

Дата: 2026-11-15. Ветка `commands`. Только чтение: код Rine не менялся, ничего не коммитилось.

Источники: V = `Paper-26.3/paper-server/src/minecraft/java/net/minecraft`. Список пакетов и id взяты из `vanilla-data/generated/reports/packets.json`, это тот же список, что регистрирует ванилла. Пути в колонке «Где в Rine» идут от `crates/rine-server/src`, если не указан другой крейт.

## Сводка

- Пакетов в протоколе 26.3: 260 (handshake, status, login, configuration, play, в обе стороны). ЕСТЬ: 129, ЧАСТИЧНО: 10, НЕТ: 121.
- Вход работает: handshake, status, офлайн-логин, сжатие, configuration (реестры, теги, known packs), переход в play. Лимиты кадров совпадают с ванилью.
- Id проверены: все константы в `rine-protocol/src/ids.rs` и локальные константы в командах совпадают с packets.json по имени и значению. Восемь констант с другим именем (например, status pong) сверены вручную.
- Игровая часть покрыта хорошо: блоки, чанки, свет, инвентарь и контейнеры, движение, предметы, TNT, команды, таб-лист, босс-бар, scoreboard, погода и граница.
- Серверная часть: белый список, баны (игроки и IP), опы, автосохранение, сохранение игроков, выгрузка чанков и трекинг есть. Нет: online mode и шифрование, RCON, Query, JSON-RPC, ресурс-паки, server.properties.
- Расхождение: `config.difficulty` (в rine.toml по умолчанию «easy») не читается, сервер стартует на normal (`cmd/commands/world.rs:259`).
- Мертвые настройки: `online_mode`, `spawn_protection`, `broadcast_rcon_to_ops`. `player_idle_timeout` только сохраняется в /setidletimeout и в тике не используется.
- `/save-off` не выключает периодическое автосохранение: флаг `st.autosave` нигде не читается.
- Опы хранятся в двух файлах (`ops.txt` и `ops.json`). Уровень ищется по нику, статус оператора по нику или по UUID, поведение смешанное.
- Обработчик правого клика по сущности (`S_INTERACT`) пустой.
- README устарел: пишет, что крафта, сундуков, редстоуна и голода нет, а в коде они есть.

## Таблица пакетов

Статусы: ЕСТЬ (пакет обрабатывается или отправляется, поле и поведение в основном есть), ЧАСТИЧНО (работает, но часть полей или поведения не сделана), НЕТ (не реализовано; пакет, который только принимается и игнорируется, тоже НЕТ). Колонка «Где в Rine» для НЕТ говорит, почему.

| Пакет | Фаза, id | Направление | Статус | Где в Rine |
|---|---|---|---|---|
| `minecraft:intention` | handshake, 0 | сервер принимает | ЧАСТИЧНО | net.rs; intent 3 (transfer) пускается как обычный вход, ванилла без accepts-transfers кикает |
| `minecraft:status_response` | status, 0 | сервер шлет | ЕСТЬ | net.rs handle_status (без favicon, sample пустой) |
| `minecraft:pong_response` | status, 1 | сервер шлет | ЕСТЬ | net.rs (C_PONG, id 1) |
| `minecraft:status_request` | status, 0 | сервер принимает | ЕСТЬ | net.rs |
| `minecraft:ping_request` | status, 1 | сервер принимает | ЕСТЬ | net.rs (S_PING, id 1) |
| `minecraft:login_disconnect` | login, 0 | сервер шлет | ЕСТЬ | net.rs (login::C_DISCONNECT) |
| `minecraft:hello` | login, 1 | сервер шлет | НЕТ | не шлется: шифрования нет (offline) |
| `minecraft:login_finished` | login, 2 | сервер шлет | ЕСТЬ | net.rs (login::C_FINISHED) |
| `minecraft:login_compression` | login, 3 | сервер шлет | ЕСТЬ | net.rs (login::C_COMPRESSION) |
| `minecraft:custom_query` | login, 4 | сервер шлет | НЕТ | не шлется |
| `minecraft:cookie_request` | login, 5 | сервер шлет | НЕТ | не шлется |
| `minecraft:hello` | login, 0 | сервер принимает | ЕСТЬ | net.rs (только ник, UUID офлайн) |
| `minecraft:key` | login, 1 | сервер принимает | НЕТ | клиент не шлет: шифрования нет |
| `minecraft:custom_query_answer` | login, 2 | сервер принимает | НЕТ | net.rs: принимается и пропускается, ответ не используется |
| `minecraft:login_acknowledged` | login, 3 | сервер принимает | ЕСТЬ | net.rs (login::S_ACKNOWLEDGED) |
| `minecraft:cookie_response` | login, 4 | сервер принимает | НЕТ | net.rs: принимается и пропускается |
| `minecraft:cookie_request` | configuration, 0 | сервер шлет | НЕТ | не шлется |
| `minecraft:custom_payload` | configuration, 1 | сервер шлет | ЕСТЬ | net.rs (brand «Rine») |
| `minecraft:disconnect` | configuration, 2 | сервер шлет | ЕСТЬ | net.rs (cfg::C_DISCONNECT) |
| `minecraft:finish_configuration` | configuration, 3 | сервер шлет | ЕСТЬ | net.rs (cfg::C_FINISH) |
| `minecraft:keep_alive` | configuration, 4 | сервер шлет | НЕТ | в конфигурации keep alive не шлется (out.rs keep_alive это play 45) |
| `minecraft:ping` | configuration, 5 | сервер шлет | НЕТ | не шлется |
| `minecraft:reset_chat` | configuration, 6 | сервер шлет | НЕТ | не шлется |
| `minecraft:registry_data` | configuration, 7 | сервер шлет | ЕСТЬ | net.rs build_config_frames (registry_data) |
| `minecraft:resource_pack_pop` | configuration, 8 | сервер шлет | НЕТ | ресурс-паки не отдаются |
| `minecraft:resource_pack_push` | configuration, 9 | сервер шлет | НЕТ | ресурс-паки не отдаются |
| `minecraft:post_effects` | configuration, 10 | сервер шлет | НЕТ | не шлется (postfx шлет play 83) |
| `minecraft:store_cookie` | configuration, 11 | сервер шлет | НЕТ | не шлется |
| `minecraft:transfer` | configuration, 12 | сервер шлет | НЕТ | не шлется (transfer шлет play 132) |
| `minecraft:update_enabled_features` | configuration, 13 | сервер шлет | ЕСТЬ | net.rs (update_enabled_features) |
| `minecraft:update_tags` | configuration, 14 | сервер шлет | ЕСТЬ | net.rs (update_tags, rine-data tags_payload) |
| `minecraft:select_known_packs` | configuration, 15 | сервер шлет | ЕСТЬ | net.rs (select_known_packs) |
| `minecraft:custom_report_details` | configuration, 16 | сервер шлет | НЕТ | не шлется |
| `minecraft:server_links` | configuration, 17 | сервер шлет | НЕТ | не шлется |
| `minecraft:clear_dialog` | configuration, 18 | сервер шлет | НЕТ | не шлется (dialog шлет play 142) |
| `minecraft:show_dialog` | configuration, 19 | сервер шлет | НЕТ | не шлется (dialog шлет play 143) |
| `minecraft:code_of_conduct` | configuration, 20 | сервер шлет | НЕТ | не шлется |
| `minecraft:client_information` | configuration, 0 | сервер принимает | ЧАСТИЧНО | net.rs: читает язык, дальность, skin; chat_mode и main_hand не используются |
| `minecraft:cookie_response` | configuration, 1 | сервер принимает | НЕТ | игнорируется (net.rs, ветка debug) |
| `minecraft:custom_payload` | configuration, 2 | сервер принимает | НЕТ | игнорируется |
| `minecraft:finish_configuration` | configuration, 3 | сервер принимает | ЕСТЬ | net.rs (cfg::S_FINISH) |
| `minecraft:keep_alive` | configuration, 4 | сервер принимает | НЕТ | net.rs: принимается и не используется, сервер в конфигурации keep alive сам не шлет |
| `minecraft:pong` | configuration, 5 | сервер принимает | НЕТ | принимается и игнорируется |
| `minecraft:resource_pack` | configuration, 6 | сервер принимает | НЕТ | принимается и игнорируется (статуса ресурс-пака нет) |
| `minecraft:select_known_packs` | configuration, 7 | сервер принимает | ЕСТЬ | net.rs (select_known_packs) |
| `minecraft:custom_click_action` | configuration, 8 | сервер принимает | НЕТ | нет |
| `minecraft:accept_code_of_conduct` | configuration, 9 | сервер принимает | НЕТ | принимается и игнорируется (net.rs) |
| `minecraft:bundle_delimiter` | play, 0 | сервер шлет | НЕТ | только ids.rs, не шлется |
| `minecraft:add_entity` | play, 1 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:animate` | play, 2 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:award_stats` | play, 3 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:block_changed_ack` | play, 4 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:block_destruction` | play, 5 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:block_entity_data` | play, 6 | сервер шлет | ЕСТЬ | cmd/cmdblock.rs, cmd/commands/data.rs |
| `minecraft:block_event` | play, 7 | сервер шлет | ЕСТЬ | game/blockinv.rs, game/redstone/mod.rs |
| `minecraft:block_update` | play, 8 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:boss_event` | play, 9 | сервер шлет | ЕСТЬ | cmd/commands/bossbar.rs |
| `minecraft:change_difficulty` | play, 10 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:chunk_batch_finished` | play, 11 | сервер шлет | ЕСТЬ | game/mod.rs |
| `minecraft:chunk_batch_start` | play, 12 | сервер шлет | ЕСТЬ | game/mod.rs |
| `minecraft:chunks_biomes` | play, 13 | сервер шлет | НЕТ | не нужен: биомы уходят внутри чанка (id 46) |
| `minecraft:clear_titles` | play, 14 | сервер шлет | ЕСТЬ | cmd/commands/chat.rs |
| `minecraft:command_suggestions` | play, 15 | сервер шлет | НЕТ | только тестовый бот |
| `minecraft:commands` | play, 16 | сервер шлет | ЕСТЬ | cmd/mod.rs |
| `minecraft:container_close` | play, 17 | сервер шлет | ЕСТЬ | game/containers.rs |
| `minecraft:container_set_content` | play, 18 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:container_set_data` | play, 19 | сервер шлет | ЕСТЬ | game/containers.rs |
| `minecraft:container_set_slot` | play, 20 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:cookie_request` | play, 21 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:cooldown` | play, 22 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:custom_chat_completions` | play, 23 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:custom_payload` | play, 24 | сервер шлет | НЕТ | не шлется в play (брендинг только в configuration) |
| `minecraft:damage_event` | play, 25 | сервер шлет | ЕСТЬ | game/interact.rs |
| `minecraft:debug/block_value` | play, 26 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:debug/chunk_value` | play, 27 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:debug/entity_value` | play, 28 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:debug/event` | play, 29 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:debug_sample` | play, 30 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:delete_chat` | play, 31 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:disconnect` | play, 32 | сервер шлет | ЕСТЬ | net.rs, out.rs |
| `minecraft:disguised_chat` | play, 33 | сервер шлет | ЕСТЬ | cmd/commands/chat.rs, scoreboard.rs (C_DISGUISED_CHAT) |
| `minecraft:entity_event` | play, 34 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:entity_position_sync` | play, 35 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:explode` | play, 36 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:add_transient_block` | play, 37 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:forget_level_chunk` | play, 38 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:game_event` | play, 39 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:game_rule_values` | play, 40 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:game_test_highlight_pos` | play, 41 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:mount_screen_open` | play, 42 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:hurt_animation` | play, 43 | сервер шлет | НЕТ | только ids.rs, не шлется |
| `minecraft:initialize_border` | play, 44 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:keep_alive` | play, 45 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:level_chunk_with_light` | play, 46 | сервер шлет | ЕСТЬ | rine-world/src/encode.rs (PACKET_ID = 46), game/world.rs:151,188 |
| `minecraft:level_event` | play, 47 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:level_particles` | play, 48 | сервер шлет | ЕСТЬ | cmd/commands/sound.rs |
| `minecraft:light_update` | play, 49 | сервер шлет | ЕСТЬ | game/world.rs |
| `minecraft:login` | play, 50 | сервер шлет | ЕСТЬ | game/mod.rs |
| `minecraft:low_disk_space_warning` | play, 51 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:map_item_data` | play, 52 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:merchant_offers` | play, 53 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:move_entity_pos` | play, 54 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:move_entity_pos_rot` | play, 55 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:move_minecart_along_track` | play, 56 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:move_entity_rot` | play, 57 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:move_vehicle` | play, 58 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:open_book` | play, 59 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:open_screen` | play, 60 | сервер шлет | ЕСТЬ | game/containers.rs |
| `minecraft:open_sign_editor` | play, 61 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:ping` | play, 62 | сервер шлет | НЕТ | не шлется (keep alive это 45) |
| `minecraft:pong_response` | play, 63 | сервер шлет | ЕСТЬ | game/handle.rs:239 (литерал 63, ответ на ping_request 38) |
| `minecraft:place_ghost_recipe` | play, 64 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:player_abilities` | play, 65 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:player_chat` | play, 66 | сервер шлет | НЕТ | только ids.rs; чат идет через system_chat (124) |
| `minecraft:player_combat_end` | play, 67 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:player_combat_enter` | play, 68 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:player_combat_kill` | play, 69 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:player_info_remove` | play, 70 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:player_info_update` | play, 71 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:player_look_at` | play, 72 | сервер шлет | ЕСТЬ | cmd/commands/teleport.rs |
| `minecraft:player_position` | play, 73 | сервер шлет | ЕСТЬ | cmd/commands/teleport.rs, out.rs |
| `minecraft:player_rotation` | play, 74 | сервер шлет | ЕСТЬ | cmd/commands/teleport.rs |
| `minecraft:recipe_book_add` | play, 75 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:recipe_book_remove` | play, 76 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:recipe_book_settings` | play, 77 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:remove_entities` | play, 78 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:remove_mob_effect` | play, 79 | сервер шлет | ЕСТЬ | cmd/commands/effect.rs |
| `minecraft:reset_score` | play, 80 | сервер шлет | ЕСТЬ | cmd/commands/scoreboard.rs |
| `minecraft:resource_pack_pop` | play, 81 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:resource_pack_push` | play, 82 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:post_effects` | play, 83 | сервер шлет | ЕСТЬ | cmd/commands/misc/postfx.rs |
| `minecraft:respawn` | play, 84 | сервер шлет | ЕСТЬ | game/interact.rs |
| `minecraft:rotate_head` | play, 85 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:section_blocks_update` | play, 86 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:select_advancements_tab` | play, 87 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:server_data` | play, 88 | сервер шлет | НЕТ | только ids.rs |
| `minecraft:set_action_bar_text` | play, 89 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:set_border_center` | play, 90 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:set_border_lerp_size` | play, 91 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:set_border_size` | play, 92 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:set_border_warning_delay` | play, 93 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:set_border_warning_distance` | play, 94 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:set_camera` | play, 95 | сервер шлет | ЕСТЬ | cmd/commands/teleport.rs |
| `minecraft:set_chunk_cache_center` | play, 96 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:set_chunk_cache_radius` | play, 97 | сервер шлет | ЕСТЬ | game/mod.rs |
| `minecraft:set_cursor_item` | play, 98 | сервер шлет | ЕСТЬ | game/containers.rs |
| `minecraft:set_default_spawn_position` | play, 99 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:set_display_objective` | play, 100 | сервер шлет | ЕСТЬ | cmd/commands/scoreboard.rs |
| `minecraft:set_entity_data` | play, 101 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:set_entity_link` | play, 102 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:set_entity_motion` | play, 103 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:set_equipment` | play, 104 | сервер шлет | ЕСТЬ | game/track.rs |
| `minecraft:set_experience` | play, 105 | сервер шлет | ЕСТЬ | cmd/commands/experience.rs |
| `minecraft:set_health` | play, 106 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:set_held_slot` | play, 107 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:set_objective` | play, 108 | сервер шлет | ЕСТЬ | cmd/commands/scoreboard.rs |
| `minecraft:set_passengers` | play, 109 | сервер шлет | НЕТ | не шлется (посадки на сущности нет) |
| `minecraft:set_player_inventory` | play, 110 | сервер шлет | НЕТ | только ids.rs |
| `minecraft:set_player_team` | play, 111 | сервер шлет | ЕСТЬ | cmd/commands/scoreboard.rs |
| `minecraft:set_score` | play, 112 | сервер шлет | ЕСТЬ | cmd/commands/scoreboard.rs |
| `minecraft:set_simulation_distance` | play, 113 | сервер шлет | ЕСТЬ | game/mod.rs |
| `minecraft:set_subtitle_text` | play, 114 | сервер шлет | ЕСТЬ | cmd/commands/chat.rs |
| `minecraft:set_time` | play, 115 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:set_title_text` | play, 116 | сервер шлет | ЕСТЬ | cmd/commands/chat.rs |
| `minecraft:set_titles_animation` | play, 117 | сервер шлет | ЕСТЬ | cmd/commands/chat.rs |
| `minecraft:sound_entity` | play, 118 | сервер шлет | НЕТ | только ids.rs |
| `minecraft:sound` | play, 119 | сервер шлет | ЕСТЬ | cmd/commands/sound.rs, out.rs |
| `minecraft:start_configuration` | play, 120 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:stop_sound` | play, 121 | сервер шлет | ЕСТЬ | cmd/commands/sound.rs |
| `minecraft:store_cookie` | play, 122 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:swing_animation` | play, 123 | сервер шлет | ЕСТЬ | cmd/commands/entities.rs, out.rs |
| `minecraft:system_chat` | play, 124 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:tab_list` | play, 125 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:tag_query` | play, 126 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:take_item_entity` | play, 127 | сервер шлет | ЕСТЬ | out.rs |
| `minecraft:teleport_entity` | play, 128 | сервер шлет | НЕТ | только ids.rs; перемещения сущностей через 54/55/35 |
| `minecraft:test_instance_block_status` | play, 129 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:ticking_state` | play, 130 | сервер шлет | ЕСТЬ | cmd/commands/world.rs |
| `minecraft:ticking_step` | play, 131 | сервер шлет | ЕСТЬ | game/mod.rs |
| `minecraft:transfer` | play, 132 | сервер шлет | ЕСТЬ | cmd/commands/players.rs (/transfer, C_TRANSFER) |
| `minecraft:update_advancements` | play, 133 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:update_attributes` | play, 134 | сервер шлет | ЕСТЬ | cmd/commands/effect.rs |
| `minecraft:update_mob_effect` | play, 135 | сервер шлет | ЕСТЬ | cmd/commands/effect.rs |
| `minecraft:update_recipes` | play, 136 | сервер шлет | НЕТ | только ids.rs |
| `minecraft:update_tags` | play, 137 | сервер шлет | НЕТ | в play не шлется (update_tags только в configuration) |
| `minecraft:projectile_power` | play, 138 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:custom_report_details` | play, 139 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:server_links` | play, 140 | сервер шлет | НЕТ | нет обработчика и отправки |
| `minecraft:waypoint` | play, 141 | сервер шлет | ЕСТЬ | cmd/commands/misc/waypoint.rs |
| `minecraft:clear_dialog` | play, 142 | сервер шлет | ЧАСТИЧНО | cmd/commands/misc/dialog.rs (только через команду) |
| `minecraft:show_dialog` | play, 143 | сервер шлет | ЧАСТИЧНО | cmd/commands/misc/dialog.rs (только через команду) |
| `minecraft:accept_teleportation` | play, 0 | сервер принимает | ЕСТЬ | game/handle.rs (S_ACCEPT_TELEPORTATION) |
| `minecraft:attack` | play, 1 | сервер принимает | ЕСТЬ | game/handle.rs (S_ATTACK) |
| `minecraft:block_entity_tag_query` | play, 2 | сервер принимает | НЕТ | нет обработчика и отправки |
| `minecraft:bundle_item_selected` | play, 3 | сервер принимает | НЕТ | нет обработчика и отправки |
| `minecraft:change_difficulty` | play, 4 | сервер принимает | НЕТ | нет обработчика и отправки |
| `minecraft:change_game_mode` | play, 5 | сервер принимает | ЕСТЬ | game/handle.rs -> actions.rs |
| `minecraft:chat_ack` | play, 6 | сервер принимает | НЕТ | принимается и игнорируется (подписей чата нет) |
| `minecraft:chat_command` | play, 7 | сервер принимает | ЕСТЬ | game/handle.rs -> cmd/mod.rs player_command |
| `minecraft:chat_command_signed` | play, 8 | сервер принимает | ЧАСТИЧНО | game/handle.rs: команда выполняется, подпись не проверяется |
| `minecraft:chat` | play, 9 | сервер принимает | ЧАСТИЧНО | game/handle.rs chat(): без подписи, убираются управляющие символы |
| `minecraft:chat_session_update` | play, 10 | сервер принимает | НЕТ | игнорируется |
| `minecraft:chunk_batch_received` | play, 11 | сервер принимает | ЕСТЬ | game/handle.rs (S_CHUNK_BATCH_RECEIVED) |
| `minecraft:client_command` | play, 12 | сервер принимает | ЧАСТИЧНО | game/handle.rs: respawn (action 0); запрос статистики (1) игнорируется |
| `minecraft:client_tick_end` | play, 13 | сервер принимает | НЕТ | игнорируется (есть только в тестовом боте) |
| `minecraft:client_information` | play, 14 | сервер принимает | ЧАСТИЧНО | game/handle.rs: дальность и skin; язык и chat_mode не используются |
| `minecraft:command_suggestion` | play, 15 | сервер принимает | НЕТ | только ids.rs, обработчика нет (нет подсказок команд) |
| `minecraft:configuration_acknowledged` | play, 16 | сервер принимает | НЕТ | не нужно: переконфигурации нет |
| `minecraft:container_button_click` | play, 17 | сервер принимает | НЕТ | только ids.rs, обработчика нет (кнопки контейнеров) |
| `minecraft:container_click` | play, 18 | сервер принимает | ЕСТЬ | game/handle.rs -> containers.rs click |
| `minecraft:container_close` | play, 19 | сервер принимает | ЕСТЬ | game/handle.rs -> containers.rs close |
| `minecraft:container_slot_state_changed` | play, 20 | сервер принимает | НЕТ | игнорируется (crafter) |
| `minecraft:cookie_response` | play, 21 | сервер принимает | НЕТ | игнорируется (default в game/handle.rs) |
| `minecraft:custom_payload` | play, 22 | сервер принимает | НЕТ | игнорируется (брендинг клиента) |
| `minecraft:debug_subscription_request` | play, 23 | сервер принимает | НЕТ | игнорируется |
| `minecraft:edit_book` | play, 24 | сервер принимает | НЕТ | нет (редактирование книг) |
| `minecraft:entity_tag_query` | play, 25 | сервер принимает | НЕТ | нет |
| `minecraft:interact` | play, 26 | сервер принимает | НЕТ | принимается, обработчик пустой (game/handle.rs: S_INTERACT => {}) |
| `minecraft:jigsaw_generate` | play, 27 | сервер принимает | НЕТ | нет |
| `minecraft:keep_alive` | play, 28 | сервер принимает | ЕСТЬ | game/handle.rs + net.rs (keep alive) |
| `minecraft:lock_difficulty` | play, 29 | сервер принимает | НЕТ | нет (смена сложности замка) |
| `minecraft:move_player_pos` | play, 30 | сервер принимает | ЕСТЬ | game/handle.rs (S_MOVE_PLAYER_POS) |
| `minecraft:move_player_pos_rot` | play, 31 | сервер принимает | ЕСТЬ | game/handle.rs (S_MOVE_PLAYER_POS_ROT) |
| `minecraft:move_player_rot` | play, 32 | сервер принимает | ЕСТЬ | game/handle.rs (S_MOVE_PLAYER_ROT) |
| `minecraft:move_player_status_only` | play, 33 | сервер принимает | ЕСТЬ | game/handle.rs (S_MOVE_PLAYER_STATUS_ONLY) |
| `minecraft:move_vehicle` | play, 34 | сервер принимает | НЕТ | нет (транспорта нет) |
| `minecraft:paddle_boat` | play, 35 | сервер принимает | НЕТ | нет (лодок нет) |
| `minecraft:pick_item_from_block` | play, 36 | сервер принимает | ЕСТЬ | game/handle.rs -> actions.rs pick_block |
| `minecraft:pick_item_from_entity` | play, 37 | сервер принимает | ЕСТЬ | game/handle.rs -> actions.rs pick_entity |
| `minecraft:ping_request` | play, 38 | сервер принимает | ЕСТЬ | game/handle.rs (ответ pong_response 63) |
| `minecraft:place_recipe` | play, 39 | сервер принимает | НЕТ | нет (книга рецептов) |
| `minecraft:player_abilities` | play, 40 | сервер принимает | ЕСТЬ | game/handle.rs (полет в creative и spectator) |
| `minecraft:player_action` | play, 41 | сервер принимает | ЕСТЬ | game/handle.rs -> interact.rs player_action |
| `minecraft:player_command` | play, 42 | сервер принимает | ЧАСТИЧНО | game/handle.rs: только спринт (action 1 и 2), остальные действия игнорируются |
| `minecraft:player_input` | play, 43 | сервер принимает | ЧАСТИЧНО | game/handle.rs: только присед (бит 0x20), прыжок и движение не используются |
| `minecraft:player_loaded` | play, 44 | сервер принимает | НЕТ | игнорируется (есть только в тестовом боте) |
| `minecraft:pong` | play, 45 | сервер принимает | НЕТ | игнорируется |
| `minecraft:punch` | play, 46 | сервер принимает | ЕСТЬ | game/handle.rs (S_PUNCH, анимация взмаха) |
| `minecraft:recipe_book_change_settings` | play, 47 | сервер принимает | НЕТ | нет |
| `minecraft:recipe_book_seen_recipe` | play, 48 | сервер принимает | НЕТ | нет |
| `minecraft:rename_item` | play, 49 | сервер принимает | НЕТ | нет (переименование в наковальне) |
| `minecraft:resource_pack` | play, 50 | сервер принимает | НЕТ | игнорируется |
| `minecraft:seen_advancements` | play, 51 | сервер принимает | НЕТ | нет |
| `minecraft:select_trade` | play, 52 | сервер принимает | НЕТ | нет (торговля с жителями) |
| `minecraft:set_beacon` | play, 53 | сервер принимает | НЕТ | нет (маяк) |
| `minecraft:set_carried_item` | play, 54 | сервер принимает | ЕСТЬ | game/handle.rs (S_SET_CARRIED_ITEM) |
| `minecraft:set_command_block` | play, 55 | сервер принимает | ЕСТЬ | game/handle.rs:85 (литерал 55) -> cmd/cmdblock.rs handle_set |
| `minecraft:set_command_minecart` | play, 56 | сервер принимает | НЕТ | нет |
| `minecraft:set_creative_mode_slot` | play, 57 | сервер принимает | ЕСТЬ | game/handle.rs (S_SET_CREATIVE_MODE_SLOT) |
| `minecraft:set_game_rule` | play, 58 | сервер принимает | НЕТ | нет (правила через команду) |
| `minecraft:set_jigsaw_block` | play, 59 | сервер принимает | НЕТ | нет |
| `minecraft:set_structure_block` | play, 60 | сервер принимает | НЕТ | нет |
| `minecraft:set_test_block` | play, 61 | сервер принимает | НЕТ | нет |
| `minecraft:sign_update` | play, 62 | сервер принимает | НЕТ | нет (таблички не редактируются) |
| `minecraft:spectator_action` | play, 63 | сервер принимает | НЕТ | нет (режим наблюдателя) |
| `minecraft:teleport_to_entity` | play, 64 | сервер принимает | НЕТ | нет |
| `minecraft:test_instance_block_action` | play, 65 | сервер принимает | НЕТ | нет |
| `minecraft:use_item_on` | play, 66 | сервер принимает | ЕСТЬ | game/handle.rs (S_USE_ITEM_ON) |
| `minecraft:use_item` | play, 67 | сервер принимает | ЕСТЬ | game/handle.rs (S_USE_ITEM) |
| `minecraft:custom_click_action` | play, 68 | сервер принимает | НЕТ | нет |

## Таблица серверных механик

| Механика | Ваниль (V) | Статус | Где в Rine | Заметки |
|---|---|---|---|---|
| Конфиг | `dedicated/DedicatedServerProperties.java, DedicatedServerSettings.java` | ЧАСТИЧНО | config.rs (rine.toml) | server.properties не читается, свой rine.toml. Часть полей мертва, см. ниже. |
| Вход: офлайн-UUID | `network/ServerLoginPacketListenerImpl.java` | ЕСТЬ | net.rs (offline_uuid, MD5 «OfflinePlayer:») | Совпадает с UUID.nameUUIDFromBytes. |
| Вход: online mode, RSA и AES/CFB8, сессия Mojang | `network/ServerLoginPacketListenerImpl.java, util/Crypt.java, CipherBase.java` | НЕТ | net.rs (комментарий «online mode is TODO») | config.online_mode есть, но нигде не читается. Скинов нет (properties пустые). |
| Сжатие пакетов | `network/CompressionDecoder.java, PacketEncoder.java` | ЕСТЬ | net.rs (login_compression), rine-protocol/src/frame.rs | Порог из rine.toml, по умолчанию 256. Лимиты кадров те же, что в ваниле: 2097151 и 8388608 байт. |
| Лимит игроков | `players/PlayerList.java` | ЕСТЬ | net.rs (max_players, «Сервер заполнен») | Проверка до сжатия. |
| Список серверов (status) | `network/ServerStatusPacketListenerImpl.java` | ЧАСТИЧНО | net.rs handle_status | Нет favicon, sample пустой. enforcesSecureChat false. |
| Legacy ping (до 1.7) | `network/LegacyQueryHandler.java` | НЕТ | - | Клиент 26.3 не нужен, но в ваниле есть. |
| Keep alive и таймаут | `network/ServerCommonPacketListenerImpl.java` | ЧАСТИЧНО | game/mod.rs second(), game/handle.rs | Rine: отправка раз в 10 с, таймаут 30 с. В файле Paper (помечен // Paper) отправка раз в секунду, лимит 30 с через свойство paper.playerconnection.keepalive. Чистая ванилла не сверена. |
| Пинг игрока в табе | `network/ServerPlayerConnection.java` | ЕСТЬ | game/handle.rs (ping_ms), game/mod.rs (info_latency) |  |
| Конфигурация: реестры, теги, known packs, features | `network/ServerConfigurationPacketListenerImpl.java` | ЕСТЬ | net.rs build_config_frames и цикл configuration | Клиент берет данные из своего встроенного пакета core. |
| Конфигурация: ресурс-паки | `network/ServerConfigurationPacketListenerImpl.java, packs/` | НЕТ | - | Не отдаются. Входящий resource_pack игнорируется. |
| Конфигурация: code of conduct | `network/protocol/configuration (code_of_conduct)` | НЕТ | - | Не отправляется. Тестовый бот входит без него. Нужен ли он клиенту 26.3, не уверен. |
| Cookies (запрос, ответ, хранение) | `network/CommonListenerCookie.java, protocol/cookie/` | НЕТ | - | Запросов не шлется. Ответы в login принимаются и игнорируются. |
| Перенос на другой сервер (transfer) | `server/network/ServerHandshakePacketListenerImpl.java (ветка TRANSFER)` | ЧАСТИЧНО | cmd/commands/players.rs (/transfer шлет пакет 132), net.rs (intent 3) | Вход по intent 3 пускается. Ванильный сервер без accepts-transfers кикает. На живом клиенте не проверял. |
| Белый список | `players/UserWhiteList.java, players/PlayerList.java` | ЕСТЬ | cmd/commands/players.rs (whitelist.json, check_login, /whitelist on/off/add/remove/list/reload) | Проверка в join() в игровом потоке, уже после configuration. Клиент входит в play и получает кик, в ваниле кик на этапе логина. |
| Баны игроков | `players/UserBanList.java` | ЕСТЬ | cmd/commands/players.rs (banned-players.json, сроки) |  |
| Баны по IP | `players/IpBanList.java` | ЕСТЬ | cmd/commands/players.rs (banned-ips.json) |  |
| Операторы | `players/ServerOpList.java` | ЧАСТИЧНО | cmd/commands/players.rs (op_level, is_op_profile, check_login), game/mod.rs (ops.txt и ops из rine.toml) | Два хранилища. Уровень из ops.json ищется по нику (без учета регистра). Статус оператора: ник в ops.txt или rine.toml, либо UUID в ops.json. Значит, ник из ops.txt дает статус оператора без проверки аккаунта. Уровень для ника из ops.txt берется из op_permission_level (1..4). |
| Вход с тем же ником (второе соединение) | `server/players/PlayerList.java (DUPLICATE_LOGIN_DISCONNECT_MESSAGE)` | НЕТ | game/mod.rs join() берет свободный слот | Ключа multiplayer.disconnect.duplicate_login в Rine нет. Старое соединение не выбивается, на живом сервере не проверял. |
| Уровни прав | `permissions/*` | ЧАСТИЧНО | cmd/commands/players.rs, config.rs (op_permission_level, function_permission_level) | Только уровни 0 до 4, именованных прав нет. |
| RCON | `server/rcon/RconThread.java, RconClient.java, RconConsoleSource.java` | НЕТ | - | Сервера нет. В config.rs есть broadcast_rcon_to_ops, он ничего не делает. |
| Query (GS4, UDP) | `server/rcon/thread/QueryThreadGs4.java` | НЕТ | - | Нет. |
| Management / JSON-RPC | `server/jsonrpc/ (65 файлов)` | НЕТ | - | Нет. |
| Консоль | `server/ConsoleInput.java` | ЕСТЬ | main.rs (stdin, уровень 4) |  |
| Остановка: stop и Ctrl+C | `server/MinecraftServer.java` | ЕСТЬ | cmd/commands/players.rs (stop), main.rs (ctrlc_hook) |  |
| Автосохранение мира | `level/ServerLevel.java, server/MinecraftServer.java` | ЧАСТИЧНО | game/mod.rs (каждые 5 мин world.save_all) | /save-off меняет только флаг st.autosave, а флаг нигде не читается. Периодическое сохранение не выключается. |
| /save-all, /save-on, /save-off | `server/commands/*` | ЧАСТИЧНО | cmd/commands/players.rs | Команды есть. /save-off без эффекта на периодическое сохранение. |
| Сохранение при остановке | `server/MinecraftServer.java` | ЕСТЬ | game/mod.rs (игроки, мир, scoreboard) |  |
| Сохранение игроков | `level/ServerPlayer.java` | ЕСТЬ | game/playerdata.rs (каждые 5 мин, при выходе, при остановке) |  |
| Загрузка и выгрузка чанков | `level/ChunkMap.java, DistanceManager.java, ChunkHolder.java, ServerChunkCache.java` | ЧАСТИЧНО | game/mod.rs stream(), game/world.rs unload_idle | Свой механизм без тикетов. Чанки без зрителей выгружаются через 30 с. Спавн (4 чанка от спавна) и forced не трогаются. При старте генерируется 13 на 13 чанков вокруг спавна. |
| Выдача чанков: батчи и квота | `level/*, network/PlayerChunkSender.java` | ЕСТЬ | game/mod.rs stream, game/handle.rs (S_CHUNK_BATCH_RECEIVED) | Квота по ack клиента, как в ваниле. |
| Дальность прорисовки клиента | `level/ClientInformation.java` | ЕСТЬ | game/handle.rs (clamp 2..view_distance) |  |
| Дистанция симуляции | `level/ServerLevel.java` | ЕСТЬ | game/entity.rs (sim_r), game/mod.rs (set_simulation_distance) |  |
| Трекинг видимости сущностей | `level/ServerEntity.java, ChunkMap.TrackedEntity` | ЧАСТИЧНО | game/track.rs, game/entity.rs (track_range) | Трекаются предметы (6 чанков), TNT и падающие блоки (10 чанков), игроки. |
| Типы сущностей | `world/entity/*` | ЧАСТИЧНО | game/entity.rs Kind: Item, Tnt, Falling | Мобов, опыта, стрел, лодок, вагонеток, картин и рамок нет. |
| Незер и Энд | `level (измерения)` | НЕТ | - | Только overworld. В HANDOFF стоит в плане (пункт 7). |
| Тайм-аут простоя (player-idle-timeout) | `players/PlayerList.java` | НЕТ | cmd/commands/players.rs (/setidletimeout сохраняет значение) | Значение в тике не читается, кика за простой нет. |
| spawn-protection | `dedicated/*` | НЕТ | config.rs (spawn_protection) | Поле не читается. |
| Сложность из конфига | `dedicated/DedicatedServerProperties.java (difficulty)` | ЧАСТИЧНО | config.rs (difficulty), cmd/commands/world.rs:259 | config.difficulty не читается, rine.toml по умолчанию пишет «easy». Сервер стартует на normal (unwrap_or(2)). /difficulty работает. |
| Режим игры по умолчанию | `dedicated/*` | ЕСТЬ | game/mod.rs (config.gamemode) |  |
| Погода, время, граница мира | `level/ServerLevel.java` | ЕСТЬ | cmd/commands/world.rs |  |
| Игровые правила | `level/* (GameRules)` | ЕСТЬ | cmd/rules.rs, cmd/commands/gamerule.rs |  |
| Спавн и возрождение | `level/PlayerSpawnFinder.java, level/ServerPlayer.java` | ЧАСТИЧНО | game/interact.rs (respawn), общий спавн | Спавн по кровати и якорю возрождения не найдены. |
| Сон (кровать) | `players/SleepStatus.java` | НЕТ | - | Обработки сна не нашел. |
| Функции (/function) | `server/ServerFunctionManager.java, ServerFunctionLibrary.java` | ЕСТЬ | cmd/commands/function.rs |  |
| Datapacks | `packs/repository/*, ServerFunctionLibrary` | ЧАСТИЧНО | cmd/commands/datapack.rs | Есть /datapack reload. Объем поддержки не проверял. |
| Advancements | `advancements/, ServerAdvancementManager.java, PlayerAdvancements.java` | НЕТ | cmd/commands/misc/advancement.rs (заглушка) | Дерева нет, пакеты не шлются. |
| Рецепты (recipe manager) | `server/ReloadableServerRegistries.java` | НЕТ | cmd/commands/misc/advancement.rs (/recipe заглушка) | update_recipes не шлется, книги рецептов нет. Крафт в контейнерах есть (game/containers.rs). |
| Чат: подписи, цепочки, отчеты | `network/chat/*, ServerGamePacketListenerImpl` | НЕТ | game/handle.rs chat() | Сообщения без подписи, рассылаются system_chat. |
| Фильтр текста | `network/TextFilter.java, ServerTextFilter.java` | НЕТ | - | Не нужен для сервера без фильтрации, в ваниле есть. |
| Подсказки команд (tab) | `network/ServerCommandSuggestionsProvider.java` | НЕТ | ids.rs (id есть), обработчика нет |  |
| Команды | `server/commands/ (111 файлов), reports/commands.json (94 корня)` | ЧАСТИЧНО | rine-command, game/cmd/ | Грубая проверка по строкам кода: из 94 корней литерал найден у 78. tp, tell и w стоят как редиректы в дереве (teleport.rs, chat.rs), test работает через gametest.rs. Исполнителей не нашел у clone, fillbiome, publish, unpublish, tm. Полного паритета по командам не проверял, см. docs/COMMANDS-PORT.md (это руководство, а не список статусов). |
| Редактирование табличек и книг | `network/protocol/game (sign, book)` | НЕТ | - | Пакеты не обрабатываются. |
| Dialog | `server/dialog/ (35 файлов)` | ЧАСТИЧНО | cmd/commands/misc/dialog.rs | Только пакеты show и clear через команду. |
| Waypoints | `server/waypoints/` | ЕСТЬ | cmd/commands/misc/waypoint.rs |  |
| Bossbar | `server/bossevents/` | ЕСТЬ | cmd/commands/bossbar.rs |  |
| Scoreboard | `server/ServerScoreboard.java` | ЕСТЬ | cmd/commands/scoreboard.rs |  |
| Серверные ссылки (server_links) | `server/ServerLinks.java` | НЕТ | - |  |
| Переконфигурация в play (start_configuration) | `network/protocol/game (start_configuration)` | НЕТ | - |  |
| Проверка версии клиента | `network/ServerHandshakePacketListenerImpl.java` | ЕСТЬ | net.rs (протокол 777, отказ с текстом) |  |
| Проверка телепорта игрока | `network/ServerGamePacketListenerImpl.java` | ЧАСТИЧНО | game/handle.rs on_move | Откат при прыжке больше 100 блоков. Остальные проверки ваниллы не портированы. |
| Правый клик по сущности (interact) | `network/ServerGamePacketListenerImpl.java (handleInteract)` | НЕТ | game/handle.rs S_INTERACT (пустой) | Сущностей с взаимодействием в Rine почти нет. |
| Лимит пакетов в секунду (rate-limit) | `server/network/ServerConnectionListener.java, network/RateKickingConnection.java, dedicated/DedicatedServerProperties.java (rate-limit, по умолчанию 0)` | НЕТ | game/handle.rs (MAX_PACKETS_PER_TICK = 400) | В ваниле лимит выключен по умолчанию. В Paper свой PacketLimiter (GlobalConfiguration, в Paper-файлах помечен // Paper). В Rine лишние пакеты ждут следующего тика, кика за спам нет. |
| Шифрование потока (Crypt, CipherBase) | `network/CipherBase.java, util/Crypt.java` | НЕТ | - | Нет, см. online mode. |
| Тики: /tick, watchdog | `dedicated/ServerWatchdog.java, server/ServerTickRateManager.java` | ЧАСТИЧНО | cmd/commands/world.rs (/tick) | Watchdog не портирован, он нужен реже. |
| Уведомления и GUI сервера | `notifications/, gui/` | НЕТ | - | Не нужно: сервер без окна. |
| Demo mode, chase | `level/DemoMode.java, chase/` | НЕТ | - | Не нужно. |
| Eula | `server/Eula.java` | НЕТ | - | Rine не проверяет eula.txt. Намеренно или нет, не уверен. |

## Покрытие групп V\server и V\network

Каждая группа V из задания отмечена, чтобы было видно, что ничего не пропущено.

| Группа | Файлов | Статус | Где в Rine | Заметки |
|---|---|---|---|---|
| server/advancements | 2 | НЕТ | - | См. Advancements в таблице механик. |
| server/bossevents | 3 | ЕСТЬ | cmd/commands/bossbar.rs |  |
| server/chase | 3 | НЕТ | - | Инструмент разработки, не нужен. |
| server/commands | 111 | ЧАСТИЧНО | rine-command, game/cmd/ | См. Команды в таблице механик. |
| server/dedicated | 7 | ЧАСТИЧНО | config.rs, main.rs | server.properties не читается. |
| server/dialog | 35 | ЧАСТИЧНО | cmd/commands/misc/dialog.rs | Только пакеты show и clear. |
| server/gui | 4 | НЕТ | - | Не нужно. |
| server/jsonrpc | 65 | НЕТ | - | Нет. |
| server/level | 42 | ЧАСТИЧНО | game/world.rs, game/mod.rs, game/track.rs, game/player.rs | Мир, чанки, игроки и сущности свои, ServerLevel и ChunkMap не портированы. |
| server/network | 29 | ЧАСТИЧНО | net.rs, game/handle.rs | Своя реализация. Подробно в таблице пакетов и механик. |
| server/notifications | 5 | НЕТ | - | Не нужно. |
| server/packs | 57 | НЕТ | - | Ресурс-паки и серверный репозиторий паков не реализованы. Datapacks: см. таблицу механик. |
| server/permissions | 12 | ЧАСТИЧНО | cmd/commands/players.rs | Только уровни 0 до 4. |
| server/players | 19 | ЧАСТИЧНО | cmd/commands/players.rs, game/playerdata.rs | Whitelist, баны, опы, сохранение игроков. Сон и idle-timeout нет. |
| server/rcon | 9 | НЕТ | - | Нет RCON и Query. |
| server/waypoints | 2 | ЕСТЬ | cmd/commands/misc/waypoint.rs |  |
| server/*.java (верхний уровень) | 27 | ЧАСТИЧНО | main.rs, game/mod.rs, game/entity.rs и др. | MinecraftServer: часть в game/mod.rs (ЧАСТИЧНО). ServerScoreboard: ЕСТЬ. ServerFunctionManager: ЕСТЬ. ServerTickRateManager: ЧАСТИЧНО. ServerLinks: НЕТ. ServerAdvancementManager: НЕТ. ReloadableServerRegistries: НЕТ (реестры берутся из rine-data). Eula: НЕТ. ConsoleInput: ЕСТЬ. Services (сервисы Mojang): НЕТ. |
| network/ (верхний уровень) | 42 | ЧАСТИЧНО | net.rs, out.rs | Свой сетевой слой на tokio. |
| network/chat/ (chat, contents, numbers) | 63 | ЧАСТИЧНО | rine-protocol/src/text.rs | Компоненты текста реализованы частично (translate, цвета). Подписей, типов чата и отчетов нет. |
| network/codec/ | 7 | ЧАСТИЧНО | rine-protocol (buf.rs, nbt.rs) | Своя реализация, сверена с ваниллой для нужных типов. |
| network/syncher/ (SynchedEntityData) | 6 | ЧАСТИЧНО | out.rs (set_entity_data), game/track.rs | Общего механизма нет, поля заполняются вручную. |
| network/protocol/ (верхний уровень) | 13 | ЧАСТИЧНО | rine-protocol/src/ids.rs, game/handle.rs, out.rs | Структура протокола своя. Id совпадают с packets.json. |
| network/protocol/common, custom, cookie, ping, status (24+4+6+6+8) | 48 | ЧАСТИЧНО | net.rs, out.rs, game/handle.rs | Keep alive, ping и status работают. Cookies и custom payload в play не шлются. |
| network/protocol/game | 198 | ЧАСТИЧНО | game/handle.rs, out.rs, game/* | Покрытие пакетов см. таблицу. |
| network/protocol/bundle (BundlePacket, BundlerInfo) | - | НЕТ | - | Бандлы не используются. По протоколу это не обязательно. |

## Самое важное, чего нет

1. Online mode и шифрование (encryption request, AES/CFB8, проверка сессии Mojang). Лицензионные клиенты на офлайн-сервер заходят, но аккаунт не проверяется: любой может зайти под любым ником, в том числе под ником из ops.txt (там сравнение по нику). Скинов нет (properties пустые). Поле `online_mode` есть, но не работает.
2. RCON и Query. Нет удаленного управления и ответа на запросы статистики.
3. Ресурс-паки: сервер их не отдает и не проверяет статус загрузки.
4. Мертвые настройки конфига: `difficulty`, `online_mode`, `spawn_protection`, `player_idle_timeout`, `broadcast_rcon_to_ops`. Их либо подключить, либо убрать из rine.toml, иначе пользователь думает, что они работают.
5. `/save-off` не отключает автосохранение.
6. Два хранилища опов (`ops.txt` и `ops.json`). Нужно одно.
7. Белый список и баны проверяются после configuration. Клиент входит в play, потом получает кик. В ваниле отказ на этапе логина.
8. Вход по intent 3 (transfer) пускается без проверки accepts-transfers.
9. Подсказки команд (tab в чате): нет обработчика и пакета ответа.
10. Чат без подписей и отчетов. Сообщения работают, но без подписи и формата 26.3.
11. Сущности: мобы, опыт, стрелы, лодки, вагонетки, картины и рамки. Трекаются только предметы, TNT и падающие блоки.
12. Блочные интерфейсы: таблички, книги, карты, торговля, рецепты, advancements, книга рецептов.
13. Незер и Энд: только overworld.
14. Сон и кровать (спавн по кровати).
15. Правый клик по сущности (`interact`) не обрабатывается.
16. Лимита пакетов в секунду нет. В ваниле он выключен по умолчанию (rate-limit = 0), в Paper есть свой PacketLimiter. Для публичного сервера это риск, не обязательный пункт протокола.
16а. Второе соединение с тем же ником не выбивает первое (сообщения duplicate_login нет).
17. Бандлы (`bundle_delimiter`, `ClientboundBundlePacket`) не используются. По протоколу это не обязательно.

## Не уверен

- Чистая ванильная частота keep alive и таймаут. Значения 1 с и 30 с взяты из файла Paper, javap по server.jar не делал.
- Нужен ли клиенту 26.3 пакет code_of_conduct. Тестовый бот входит без него, на живом клиенте не проверял.
- Поведение входа по transfer на живом клиенте.
- Datapacks: объем поддержки. Есть /datapack reload, остальное не смотрел.
- Команды: сравнение по строкам кода, не по дереву команд. Литерал не найден у части корней, хотя часть из них реализована через редиректы и шаблоны. Полный список по каждой команде не делал.

## Как проверял

- Список файлов V\server (432) и V\network (416) снят через find, в группы включены все подпапки.
- Пакеты: по packets.json (260 записей, фазы и направления). Для каждой записи искал константу из ids.rs и локальные константы по имени, отдельно искал литеральные id (`packet(63)`, `55 =>`, `PACKET_ID = 46`).
- Статус «ЕСТЬ» ставлю только если пакет реально собирается или разбирается в коде Rine, а не только объявлен в ids.rs. Пакеты, которые есть только в тестовом боте, отмечены как НЕТ.
- Механики: поиск по ключевым словам в crates/rine-server/src и чтение нужных функций (net.rs, game/mod.rs, game/handle.rs, players.rs, track.rs, world.rs).
- Ничего не запускалось: проверки на живом сервере и клиенте не делались.

## Хвосты из отчёта 01 (блоки), 2026-11-16

- Долгие тики при массовом отключении игроков: при уходе 50 ботов фаза `flush` занимала 61-950 мс за тик (замер 2026-11-16, release, порт 25571). Блоки тут ни при чём, но это видно в каждом прогоне.
- `level_event` с исключением автора: частицы воска, очистки меди и костной муки уходят всем, а ваниль не шлёт их самому игроку (`levelEvent(player, ...)`).
- Пакеты редактора табличек (`open_sign_editor`, `sign_update`) и выхода из кровати добавляются в отчёте 01 (`signs.rs`, `bed.rs`).
