//! Compile-time inventory for the Lua files shared by player and renderer
//! installations. Lovely module declarations are validated against this list.

pub const SHARED_MOD_PAYLOAD: &[(&str, &[u8])] = &[
    (
        "bbt/core.lua",
        include_bytes!("../../mod/shared/bbt/core.lua"),
    ),
    (
        "bbt/dashboard_model.lua",
        include_bytes!("../../mod/shared/bbt/dashboard_model.lua"),
    ),
    (
        "bbt/ipc_thread.lua",
        include_bytes!("../../mod/shared/bbt/ipc_thread.lua"),
    ),
    (
        "bbt/online_state.lua",
        include_bytes!("../../mod/shared/bbt/online_state.lua"),
    ),
    (
        "bbt/renderer.lua",
        include_bytes!("../../mod/shared/bbt/renderer.lua"),
    ),
    (
        "lovely/hooks.toml",
        include_bytes!("../../mod/shared/lovely/hooks.toml"),
    ),
];
