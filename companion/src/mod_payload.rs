//! Compile-time inventory for the Lua files shared by player and renderer
//! installations. Lovely module declarations are validated against this list.

pub const SHARED_MOD_PAYLOAD: &[(&str, &[u8])] = &[
    (
        "assets/online.png",
        include_bytes!("../../mod/shared/assets/online.png"),
    ),
    (
        "bbt/core.lua",
        include_bytes!("../../mod/shared/bbt/core.lua"),
    ),
    (
        "bbt/dashboard_model.lua",
        include_bytes!("../../mod/shared/bbt/dashboard_model.lua"),
    ),
    (
        "bbt/dashboard_components.lua",
        include_bytes!("../../mod/shared/bbt/dashboard_components.lua"),
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

#[cfg(test)]
mod tests {
    use super::SHARED_MOD_PAYLOAD;

    #[test]
    fn online_icon_is_embedded_at_native_menu_size() {
        let (_, icon) = SHARED_MOD_PAYLOAD
            .iter()
            .find(|(path, _)| *path == "assets/online.png")
            .expect("online icon missing from installer payload");
        assert_eq!(&icon[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(u32::from_be_bytes(icon[16..20].try_into().unwrap()), 72);
        assert_eq!(u32::from_be_bytes(icon[20..24].try_into().unwrap()), 72);
    }
}
