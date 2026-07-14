use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub version: u8,
    #[serde(rename = "type")]
    pub kind: String,
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameplayState {
    pub state: String,
    pub player_name: String,
    pub song_name: String,
    pub lobby_name: String,
    pub accuracy: f64,
    pub combo: u64,
    pub misses: u64,
    pub rank: u64,
    pub progress: f64,
    pub connected: bool,
    pub updated_at_ms: u64,
}

impl Default for GameplayState {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            player_name: "Player".into(),
            song_name: "No chart".into(),
            lobby_name: "Offline practice".into(),
            accuracy: 100.0,
            combo: 0,
            misses: 0,
            rank: 1,
            progress: 0.0,
            connected: false,
            updated_at_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionConfig {
    pub instance_url: Option<String>,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RedeemRequest {
    pub instance_url: String,
    pub invite_code: String,
    pub display_name: String,
    pub device_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartHashRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpectateRequest {
    pub lobby_id: String,
}
