use serde::{Deserialize, Serialize};

/// Метаданные текущего трека
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrackMeta {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub cover_url: Option<String>,
}

fn default_volume() -> f32 {
    0.7
}

fn default_repeat() -> String {
    "off".to_string()
}

/// Состояние плеера на колонке, передаваемое серверу
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerState {
    #[serde(default)]
    pub is_playing: bool,
    #[serde(default)]
    pub current_track: Option<TrackMeta>,
    #[serde(default)]
    pub position_ms: u64,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default = "default_volume")]
    pub volume: f32,
    #[serde(default)]
    pub is_muted: bool,
    #[serde(default = "default_repeat")]
    pub repeat_mode: String,
    #[serde(default)]
    pub is_shuffle: bool,
    #[serde(default)]
    pub queue: Vec<TrackMeta>,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            is_playing: false,
            current_track: None,
            position_ms: 0,
            duration_ms: 0,
            volume: 0.7,
            is_muted: false,
            repeat_mode: "off".to_string(),
            is_shuffle: false,
            queue: Vec::new(),
        }
    }
}

/// Исходящее сообщение от колонки к серверу
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum OutgoingMessage {
    #[serde(rename = "state_update")]
    StateUpdate { state: PlayerState },

    #[serde(rename = "ping")]
    Ping,

    #[serde(rename = "request_devices")]
    RequestDevices,
}

/// Входящее сообщение от сервера к колонке
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum IncomingMessage {
    #[serde(rename = "welcome")]
    Welcome {
        device_id: String,
        devices: Vec<serde_json::Value>,
    },

    #[serde(rename = "command")]
    Command {
        from_device_id: Option<String>,
        command: String,
        #[serde(default)]
        payload: serde_json::Value,
    },

    #[serde(rename = "devices_update")]
    DevicesUpdate {
        devices: Vec<serde_json::Value>,
    },

    #[serde(rename = "device_state")]
    DeviceState {
        device_id: String,
        device: serde_json::Value,
    },

    #[serde(rename = "pong")]
    Pong {
        #[serde(default)]
        time: Option<f64>,
    },

    #[serde(rename = "error")]
    Error {
        message: String,
    },

    #[serde(other)]
    Unknown,
}

/// Разобранная команда управления для аудиоплеера
#[derive(Debug, Clone)]
pub enum PlayerCommand {
    Play,
    Pause,
    TogglePlay,
    Stop,
    Next,
    Prev,
    Seek { position_ms: u64 },
    SetVolume { volume: f32 },
    ToggleShuffle,
    SetShuffle { enabled: bool },
    SetRepeat { mode: String },
    PlayTrack {
        track_id: i64,
        title: String,
        artist: String,
        duration_ms: u64,
        position_ms: u64,
        auto_play: bool,
        queue: Vec<TrackMeta>,
    },
}
