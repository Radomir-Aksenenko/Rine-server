//! Generated from vanilla-data/generated/reports/packets.json (play clientbound) and
//! registries.json (particle_type) plus Paper ParticleTypes.java (SimpleParticleType registrations).

/// Name of a clientbound play packet id, or "?" when unknown.
pub fn play_name(id: i32) -> &'static str {
    PLAY_NAMES.get(id as usize).copied().filter(|_| id >= 0).unwrap_or("?")
}

pub const PLAY_NAMES: [&str; 144] = [
    "bundle_delimiter",
    "add_entity",
    "animate",
    "award_stats",
    "block_changed_ack",
    "block_destruction",
    "block_entity_data",
    "block_event",
    "block_update",
    "boss_event",
    "change_difficulty",
    "chunk_batch_finished",
    "chunk_batch_start",
    "chunks_biomes",
    "clear_titles",
    "command_suggestions",
    "commands",
    "container_close",
    "container_set_content",
    "container_set_data",
    "container_set_slot",
    "cookie_request",
    "cooldown",
    "custom_chat_completions",
    "custom_payload",
    "damage_event",
    "debug/block_value",
    "debug/chunk_value",
    "debug/entity_value",
    "debug/event",
    "debug_sample",
    "delete_chat",
    "disconnect",
    "disguised_chat",
    "entity_event",
    "entity_position_sync",
    "explode",
    "add_transient_block",
    "forget_level_chunk",
    "game_event",
    "game_rule_values",
    "game_test_highlight_pos",
    "mount_screen_open",
    "hurt_animation",
    "initialize_border",
    "keep_alive",
    "level_chunk_with_light",
    "level_event",
    "level_particles",
    "light_update",
    "login",
    "low_disk_space_warning",
    "map_item_data",
    "merchant_offers",
    "move_entity_pos",
    "move_entity_pos_rot",
    "move_minecart_along_track",
    "move_entity_rot",
    "move_vehicle",
    "open_book",
    "open_screen",
    "open_sign_editor",
    "ping",
    "pong_response",
    "place_ghost_recipe",
    "player_abilities",
    "player_chat",
    "player_combat_end",
    "player_combat_enter",
    "player_combat_kill",
    "player_info_remove",
    "player_info_update",
    "player_look_at",
    "player_position",
    "player_rotation",
    "recipe_book_add",
    "recipe_book_remove",
    "recipe_book_settings",
    "remove_entities",
    "remove_mob_effect",
    "reset_score",
    "resource_pack_pop",
    "resource_pack_push",
    "post_effects",
    "respawn",
    "rotate_head",
    "section_blocks_update",
    "select_advancements_tab",
    "server_data",
    "set_action_bar_text",
    "set_border_center",
    "set_border_lerp_size",
    "set_border_size",
    "set_border_warning_delay",
    "set_border_warning_distance",
    "set_camera",
    "set_chunk_cache_center",
    "set_chunk_cache_radius",
    "set_cursor_item",
    "set_default_spawn_position",
    "set_display_objective",
    "set_entity_data",
    "set_entity_link",
    "set_entity_motion",
    "set_equipment",
    "set_experience",
    "set_health",
    "set_held_slot",
    "set_objective",
    "set_passengers",
    "set_player_inventory",
    "set_player_team",
    "set_score",
    "set_simulation_distance",
    "set_subtitle_text",
    "set_time",
    "set_title_text",
    "set_titles_animation",
    "sound_entity",
    "sound",
    "start_configuration",
    "stop_sound",
    "store_cookie",
    "swing_animation",
    "system_chat",
    "tab_list",
    "tag_query",
    "take_item_entity",
    "teleport_entity",
    "test_instance_block_status",
    "ticking_state",
    "ticking_step",
    "transfer",
    "update_advancements",
    "update_attributes",
    "update_mob_effect",
    "update_recipes",
    "update_tags",
    "projectile_power",
    "custom_report_details",
    "server_links",
    "waypoint",
    "clear_dialog",
    "show_dialog",
];

/// Particle type registry ids (protocol order) that are SimpleParticleType (no payload on the wire).
pub const PARTICLE_SIMPLE: [bool; 128] = [
    true, // 0 minecraft:angry_villager
    false, // 1 minecraft:block
    false, // 2 minecraft:block_marker
    true, // 3 minecraft:bubble
    true, // 4 minecraft:sulfur_bubbles
    true, // 5 minecraft:noxious_gas
    true, // 6 minecraft:noxious_gas_cloud
    false, // 7 minecraft:geyser
    false, // 8 minecraft:geyser_base
    false, // 9 minecraft:geyser_poof
    false, // 10 minecraft:geyser_plume
    true, // 11 minecraft:cloud
    true, // 12 minecraft:copper_fire_flame
    true, // 13 minecraft:crit
    true, // 14 minecraft:damage_indicator
    false, // 15 minecraft:dragon_breath
    true, // 16 minecraft:dripping_lava
    true, // 17 minecraft:falling_lava
    true, // 18 minecraft:landing_lava
    true, // 19 minecraft:dripping_water
    true, // 20 minecraft:falling_water
    false, // 21 minecraft:dust
    false, // 22 minecraft:dust_color_transition
    false, // 23 minecraft:effect
    true, // 24 minecraft:elder_guardian
    true, // 25 minecraft:enchanted_hit
    true, // 26 minecraft:enchant
    true, // 27 minecraft:end_rod
    false, // 28 minecraft:entity_effect
    true, // 29 minecraft:explosion_emitter
    true, // 30 minecraft:explosion
    true, // 31 minecraft:gust
    true, // 32 minecraft:small_gust
    true, // 33 minecraft:gust_emitter_large
    true, // 34 minecraft:gust_emitter_small
    true, // 35 minecraft:sonic_boom
    false, // 36 minecraft:falling_dust
    true, // 37 minecraft:firework
    true, // 38 minecraft:fishing
    true, // 39 minecraft:flame
    true, // 40 minecraft:infested
    true, // 41 minecraft:cherry_leaves
    true, // 42 minecraft:pale_oak_leaves
    true, // 43 minecraft:red_poplar_leaves
    true, // 44 minecraft:orange_poplar_leaves
    true, // 45 minecraft:yellow_poplar_leaves
    false, // 46 minecraft:tinted_leaves
    true, // 47 minecraft:sculk_soul
    false, // 48 minecraft:sculk_charge
    true, // 49 minecraft:sculk_charge_pop
    true, // 50 minecraft:soul_fire_flame
    true, // 51 minecraft:soul
    false, // 52 minecraft:flash
    true, // 53 minecraft:happy_villager
    true, // 54 minecraft:composter
    true, // 55 minecraft:heart
    false, // 56 minecraft:instant_effect
    false, // 57 minecraft:item
    false, // 58 minecraft:vibration
    false, // 59 minecraft:trail
    true, // 60 minecraft:pause_mob_growth
    true, // 61 minecraft:reset_mob_growth
    true, // 62 minecraft:item_slime
    true, // 63 minecraft:item_cobweb
    true, // 64 minecraft:item_snowball
    true, // 65 minecraft:large_smoke
    true, // 66 minecraft:lava
    true, // 67 minecraft:mycelium
    true, // 68 minecraft:note
    true, // 69 minecraft:poof
    true, // 70 minecraft:portal
    true, // 71 minecraft:rain
    true, // 72 minecraft:smoke
    true, // 73 minecraft:white_smoke
    true, // 74 minecraft:sneeze
    true, // 75 minecraft:spit
    true, // 76 minecraft:squid_ink
    true, // 77 minecraft:sweep_attack
    true, // 78 minecraft:totem_of_undying
    true, // 79 minecraft:underwater
    true, // 80 minecraft:splash
    true, // 81 minecraft:witch
    true, // 82 minecraft:bubble_pop
    true, // 83 minecraft:current_down
    true, // 84 minecraft:bubble_column_up
    true, // 85 minecraft:nautilus
    true, // 86 minecraft:dolphin
    true, // 87 minecraft:campfire_cosy_smoke
    true, // 88 minecraft:campfire_signal_smoke
    true, // 89 minecraft:dripping_honey
    true, // 90 minecraft:falling_honey
    true, // 91 minecraft:landing_honey
    true, // 92 minecraft:falling_nectar
    true, // 93 minecraft:falling_spore_blossom
    true, // 94 minecraft:ash
    true, // 95 minecraft:crimson_spore
    true, // 96 minecraft:warped_spore
    true, // 97 minecraft:spore_blossom_air
    true, // 98 minecraft:dripping_obsidian_tear
    true, // 99 minecraft:falling_obsidian_tear
    true, // 100 minecraft:landing_obsidian_tear
    true, // 101 minecraft:reverse_portal
    true, // 102 minecraft:white_ash
    true, // 103 minecraft:small_flame
    true, // 104 minecraft:snowflake
    true, // 105 minecraft:dripping_dripstone_lava
    true, // 106 minecraft:falling_dripstone_lava
    true, // 107 minecraft:dripping_dripstone_water
    true, // 108 minecraft:falling_dripstone_water
    true, // 109 minecraft:glow_squid_ink
    true, // 110 minecraft:glow
    true, // 111 minecraft:wax_on
    true, // 112 minecraft:wax_off
    true, // 113 minecraft:electric_spark
    true, // 114 minecraft:scrape
    false, // 115 minecraft:shriek
    true, // 116 minecraft:egg_crack
    true, // 117 minecraft:dust_plume
    true, // 118 minecraft:trial_spawner_detection
    true, // 119 minecraft:trial_spawner_detection_ominous
    true, // 120 minecraft:vault_connection
    false, // 121 minecraft:dust_pillar
    true, // 122 minecraft:ominous_spawning
    true, // 123 minecraft:raid_omen
    true, // 124 minecraft:trial_omen
    false, // 125 minecraft:block_crumble
    true, // 126 minecraft:firefly
    true, // 127 minecraft:sulfur_cube_goo
];

