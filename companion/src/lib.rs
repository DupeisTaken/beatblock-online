pub mod app_state;
pub mod chart_hash;
pub mod exports;
pub mod game_commands;
#[cfg(feature = "installer-ui")]
pub mod gui;
pub mod http;
pub mod installer;
pub mod ipc;
pub mod journal;
pub mod mod_payload;
pub mod model;
pub mod nat;
pub mod network;
pub mod renderer;
pub mod room;
pub mod storage;

// Archive-transfer code is quarantined to its security tests until an
// authenticated transport and explicit in-game consent flow are implemented.
// Production advertises verify-only charts and cannot call this module.
#[cfg(test)]
mod transfer;
