//! Packet ids for protocol 777, taken from the vanilla 26.3 data generator
//! report (generated/reports/packets.json). Only ids we use are listed.

pub mod handshake {
    pub const INTENTION: i32 = 0;
}

pub mod status {
    pub const C_STATUS_RESPONSE: i32 = 0;
    pub const C_PONG: i32 = 1;
    pub const S_STATUS_REQUEST: i32 = 0;
    pub const S_PING: i32 = 1;
}

pub mod login {
    pub const C_DISCONNECT: i32 = 0;
    pub const C_HELLO: i32 = 1;
    pub const C_FINISHED: i32 = 2;
    pub const C_COMPRESSION: i32 = 3;
    pub const C_CUSTOM_QUERY: i32 = 4;
    pub const S_HELLO: i32 = 0;
    pub const S_KEY: i32 = 1;
    pub const S_CUSTOM_QUERY_ANSWER: i32 = 2;
    pub const S_ACKNOWLEDGED: i32 = 3;
    pub const S_COOKIE_RESPONSE: i32 = 4;
}

pub mod config {
    pub const C_CUSTOM_PAYLOAD: i32 = 1;
    pub const C_DISCONNECT: i32 = 2;
    pub const C_FINISH: i32 = 3;
    pub const C_KEEP_ALIVE: i32 = 4;
    pub const C_REGISTRY_DATA: i32 = 7;
    pub const C_UPDATE_ENABLED_FEATURES: i32 = 13;
    pub const C_UPDATE_TAGS: i32 = 14;
    pub const C_SELECT_KNOWN_PACKS: i32 = 15;

    pub const S_CLIENT_INFORMATION: i32 = 0;
    pub const S_CUSTOM_PAYLOAD: i32 = 2;
    pub const S_FINISH: i32 = 3;
    pub const S_KEEP_ALIVE: i32 = 4;
    pub const S_PONG: i32 = 5;
    pub const S_RESOURCE_PACK: i32 = 6;
    pub const S_SELECT_KNOWN_PACKS: i32 = 7;
    pub const S_ACCEPT_CODE_OF_CONDUCT: i32 = 9;
}

pub mod play {
    // clientbound
    pub const C_BUNDLE_DELIMITER: i32 = 0;
    pub const C_ADD_ENTITY: i32 = 1;
    pub const C_ANIMATE: i32 = 2;
    pub const C_SWING_ANIMATION: i32 = 123;
    pub const C_BLOCK_CHANGED_ACK: i32 = 4;
    pub const C_BLOCK_DESTRUCTION: i32 = 5;
    pub const C_BLOCK_ENTITY_DATA: i32 = 6;
    pub const C_BLOCK_EVENT: i32 = 7;
    pub const C_BLOCK_UPDATE: i32 = 8;
    pub const C_CHANGE_DIFFICULTY: i32 = 10;
    pub const C_CHUNK_BATCH_FINISHED: i32 = 11;
    pub const C_CHUNK_BATCH_START: i32 = 12;
    pub const C_COMMAND_SUGGESTIONS: i32 = 15;
    pub const C_COMMANDS: i32 = 16;
    pub const C_CONTAINER_CLOSE: i32 = 17;
    pub const C_CONTAINER_SET_CONTENT: i32 = 18;
    pub const C_CONTAINER_SET_DATA: i32 = 19;
    pub const C_CONTAINER_SET_SLOT: i32 = 20;
    pub const C_DAMAGE_EVENT: i32 = 25;
    pub const C_DISCONNECT: i32 = 32;
    pub const C_ENTITY_EVENT: i32 = 34;
    pub const C_ENTITY_POSITION_SYNC: i32 = 35;
    pub const C_EXPLODE: i32 = 36;
    pub const C_FORGET_LEVEL_CHUNK: i32 = 38;
    pub const C_GAME_EVENT: i32 = 39;
    pub const C_HURT_ANIMATION: i32 = 43;
    pub const C_INITIALIZE_BORDER: i32 = 44;
    pub const C_KEEP_ALIVE: i32 = 45;
    pub const C_LEVEL_CHUNK_WITH_LIGHT: i32 = 46;
    pub const C_LEVEL_EVENT: i32 = 47;
    pub const C_LEVEL_PARTICLES: i32 = 48;
    pub const C_LIGHT_UPDATE: i32 = 49;
    pub const C_LOGIN: i32 = 50;
    pub const C_MOVE_ENTITY_POS: i32 = 54;
    pub const C_MOVE_ENTITY_POS_ROT: i32 = 55;
    pub const C_MOVE_ENTITY_ROT: i32 = 57;
    pub const C_OPEN_SCREEN: i32 = 60;
    pub const C_OPEN_SIGN_EDITOR: i32 = 61;
    pub const C_PLAYER_ABILITIES: i32 = 65;
    pub const C_PLAYER_CHAT: i32 = 66;
    pub const C_PLAYER_INFO_REMOVE: i32 = 70;
    pub const C_PLAYER_INFO_UPDATE: i32 = 71;
    pub const C_PLAYER_POSITION: i32 = 73;
    pub const C_PLAYER_ROTATION: i32 = 74;
    pub const C_REMOVE_ENTITIES: i32 = 78;
    pub const C_RESPAWN: i32 = 84;
    pub const C_ROTATE_HEAD: i32 = 85;
    pub const C_SECTION_BLOCKS_UPDATE: i32 = 86;
    pub const C_SERVER_DATA: i32 = 88;
    pub const C_SET_ACTION_BAR_TEXT: i32 = 89;
    pub const C_SET_CHUNK_CACHE_CENTER: i32 = 96;
    pub const C_SET_CHUNK_CACHE_RADIUS: i32 = 97;
    pub const C_SET_CURSOR_ITEM: i32 = 98;
    pub const C_SET_DEFAULT_SPAWN_POSITION: i32 = 99;
    pub const C_SET_ENTITY_DATA: i32 = 101;
    pub const C_SET_ENTITY_MOTION: i32 = 103;
    pub const C_SET_EQUIPMENT: i32 = 104;
    pub const C_SET_EXPERIENCE: i32 = 105;
    pub const C_SET_HEALTH: i32 = 106;
    pub const C_SET_HELD_SLOT: i32 = 107;
    pub const C_SET_PLAYER_INVENTORY: i32 = 110;
    pub const C_SET_SIMULATION_DISTANCE: i32 = 113;
    pub const C_SET_TIME: i32 = 115;
    pub const C_SOUND_ENTITY: i32 = 118;
    pub const C_SOUND: i32 = 119;
    pub const C_SYSTEM_CHAT: i32 = 124;
    pub const C_TAB_LIST: i32 = 125;
    pub const C_TAKE_ITEM_ENTITY: i32 = 127;
    pub const C_TELEPORT_ENTITY: i32 = 128;
    pub const C_TICKING_STATE: i32 = 130;
    pub const C_TICKING_STEP: i32 = 131;
    pub const C_UPDATE_ATTRIBUTES: i32 = 134;
    pub const C_UPDATE_RECIPES: i32 = 136;

    // serverbound
    pub const S_ACCEPT_TELEPORTATION: i32 = 0;
    pub const S_ATTACK: i32 = 1;
    pub const S_CHANGE_GAME_MODE: i32 = 5;
    pub const S_CHAT: i32 = 9;
    pub const S_CHAT_ACK: i32 = 6;
    pub const S_CHAT_COMMAND: i32 = 7;
    pub const S_CHAT_COMMAND_SIGNED: i32 = 8;
    pub const S_CHAT_SESSION_UPDATE: i32 = 10;
    pub const S_CHUNK_BATCH_RECEIVED: i32 = 11;
    pub const S_CLIENT_COMMAND: i32 = 12;
    pub const S_CLIENT_TICK_END: i32 = 13;
    pub const S_CLIENT_INFORMATION: i32 = 14;
    pub const S_COMMAND_SUGGESTION: i32 = 15;
    pub const S_CONTAINER_BUTTON_CLICK: i32 = 17;
    pub const S_CONTAINER_CLICK: i32 = 18;
    pub const S_CONTAINER_CLOSE: i32 = 19;
    pub const S_CUSTOM_PAYLOAD: i32 = 22;
    pub const S_INTERACT: i32 = 26;
    pub const S_KEEP_ALIVE: i32 = 28;
    pub const S_MOVE_PLAYER_POS: i32 = 30;
    pub const S_MOVE_PLAYER_POS_ROT: i32 = 31;
    pub const S_MOVE_PLAYER_ROT: i32 = 32;
    pub const S_MOVE_PLAYER_STATUS_ONLY: i32 = 33;
    pub const S_PICK_ITEM_FROM_BLOCK: i32 = 36;
    pub const S_PICK_ITEM_FROM_ENTITY: i32 = 37;
    pub const S_PING_REQUEST: i32 = 38;
    pub const S_PLAYER_ABILITIES: i32 = 40;
    pub const S_PLAYER_ACTION: i32 = 41;
    pub const S_PLAYER_COMMAND: i32 = 42;
    pub const S_PLAYER_INPUT: i32 = 43;
    pub const S_PLAYER_LOADED: i32 = 44;
    pub const S_PONG: i32 = 45;
    pub const S_PUNCH: i32 = 46;
    pub const S_SET_CARRIED_ITEM: i32 = 54;
    pub const S_SET_CREATIVE_MODE_SLOT: i32 = 57;
    pub const S_SIGN_UPDATE: i32 = 62;
    pub const S_USE_ITEM_ON: i32 = 66;
    pub const S_USE_ITEM: i32 = 67;
}
