use serde::{de::{self, Deserializer, Visitor}, Deserialize, Serialize};

/// Гибкая десериализация ID трека (поддерживает как целые числа, так и строки кандидатов вроде "candidate_hitmo_...")
pub fn deserialize_flexible_id<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    struct IdVisitor;

    impl<'de> Visitor<'de> for IdVisitor {
        type Value = i64;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("an integer or string representing a track ID")
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value)
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value as i64)
        }

        fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(value as i64)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            if let Ok(num) = value.parse::<i64>() {
                Ok(num)
            } else {
                // Строковые ID предпросмотров кандидатов (напр. "candidate_...")
                Ok(-1)
            }
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(0)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            Ok(0)
        }

        fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
        where
            D2: Deserializer<'de>,
        {
            deserializer.deserialize_any(self)
        }
    }

    deserializer.deserialize_any(IdVisitor)
}

/// Метаданные текущего трека
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TrackMeta {
    #[serde(default, deserialize_with = "deserialize_flexible_id")]
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
    #[serde(default)]
    pub is_preview: bool,
    #[serde(default)]
    pub candidate_id: Option<String>,
    #[serde(default)]
    pub is_liked: bool,
    #[serde(default)]
    pub is_disliked: bool,
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
    pub is_flow: bool,
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
            is_flow: false,
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
        is_preview: bool,
        candidate_id: Option<String>,
        cover_url: Option<String>,
        is_liked: bool,
        is_disliked: bool,
    },
    SetQueue {
        queue: Vec<TrackMeta>,
        track_id: i64,
    },
    SetReaction {
        track_id: i64,
        is_liked: bool,
        is_disliked: bool,
    },
    StartFlow,
    Duck { ducked: bool },
}
