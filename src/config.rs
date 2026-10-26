use std::fs;
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationConfig {
    /// URL сервера Sentinel Music (например, http://192.168.1.100:8000)
    pub server_url: String,

    /// Токен авторизации (API токен пользователя)
    pub auth_token: String,

    /// Уникальный идентификатор устройства (если не указан, генерируется из hostname/MAC)
    #[serde(default = "default_device_id")]
    pub device_id: String,

    /// Отображаемое имя колонки в приложениях (Sentinel Connect)
    #[serde(default = "default_device_name")]
    pub device_name: String,

    /// Тип устройства (для сервера: player, remote, hybrid)
    #[serde(default = "default_device_type")]
    pub device_type: String,

    /// Имя аудиоустройства ALSA (например: "default", "plughw:0,0")
    #[serde(default = "default_audio_device")]
    pub audio_device: String,

    /// Начальный уровень громкости (от 0.0 до 1.0)
    #[serde(default = "default_volume")]
    pub initial_volume: f32,

    /// Включить визуализатор спектра (FFT)
    #[serde(default = "default_true")]
    pub visualizer_enabled: bool,

    /// Путь к интерфейсу аппаратной LED-матрицы Станции (например /dev/spidev0.0 или /dev/matrix)
    pub led_matrix_path: Option<String>,

    /// Автоматическое переподключение при разрыве связи
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,

    /// Таймаут пинга в секундах
    #[serde(default = "default_ping_interval")]
    pub ping_interval_secs: u64,

    /// Включить обработку сенсорных кнопок на корпусе колонки
    #[serde(default = "default_true")]
    pub buttons_enabled: bool,

    /// Путь к файлу событий кнопок в системе (evdev)
    #[serde(default = "default_input_device")]
    pub input_device_path: String,

    /// Режим вывода звука: "auto" (наушники при подключении кабеля), "both" (и динамик, и AUX), "speaker", "aux"
    #[serde(default = "default_audio_routing")]
    pub audio_routing: String,

    /// Яркость LED-экрана Станции (от 0 до 255)
    #[serde(default = "default_brightness")]
    pub display_brightness: u8,

    /// Смещение часового пояса в часах для часов (по умолчанию +3 для Москвы)
    #[serde(default = "default_timezone_offset")]
    pub timezone_offset_hours: i32,
}

fn default_brightness() -> u8 {
    120
}

fn default_timezone_offset() -> i32 {
    3
}

fn default_audio_routing() -> String {
    "auto".to_string()
}

fn default_input_device() -> String {
    "/dev/input/event0".to_string()
}

fn default_device_id() -> String {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "station".to_string());
    format!("sentinel-station-{}", hostname)
}

fn default_device_name() -> String {
    "Яндекс.Станция (Sentinel)".to_string()
}

fn default_device_type() -> String {
    "player".to_string()
}

fn default_audio_device() -> String {
    "default".to_string()
}

fn default_volume() -> f32 {
    0.7
}

fn default_true() -> bool {
    true
}

fn default_ping_interval() -> u64 {
    15
}

impl Default for StationConfig {
    fn default() -> Self {
        Self {
            server_url: "http://192.168.1.100:8000".to_string(),
            auth_token: "".to_string(),
            device_id: default_device_id(),
            device_name: default_device_name(),
            device_type: default_device_type(),
            audio_device: default_audio_device(),
            initial_volume: default_volume(),
            visualizer_enabled: true,
            led_matrix_path: None,
            auto_reconnect: true,
            ping_interval_secs: 15,
            buttons_enabled: true,
            input_device_path: default_input_device(),
            audio_routing: default_audio_routing(),
            display_brightness: default_brightness(),
            timezone_offset_hours: default_timezone_offset(),
        }
    }
}

impl StationConfig {
    /// Поиск и загрузка файла конфигурации
    pub fn load(explicit_path: Option<&Path>) -> Result<Self> {
        let candidate_paths = [
            explicit_path.map(|p| p.to_path_buf()),
            Some(PathBuf::from("config.toml")),
            Some(PathBuf::from("/data/opt/sentinel-station/config.toml")),
            Some(PathBuf::from("/etc/sentinel-station/config.toml")),
        ];

        for opt_path in candidate_paths.into_iter().flatten() {
            if opt_path.exists() {
                info!("Загрузка конфигурации из: {}", opt_path.display());
                let content = fs::read_to_string(&opt_path)
                    .with_context(|| format!("Не удалось прочитать файл {}", opt_path.display()))?;
                let config: StationConfig = toml::from_str(&content)
                    .with_context(|| format!("Ошибка разбора TOML в {}", opt_path.display()))?;
                return Ok(config);
            }
        }

        // Если файл не найден, создаем шаблон по умолчанию
        let default_cfg = StationConfig::default();
        let target_path = explicit_path.unwrap_or_else(|| Path::new("config.toml"));
        if let Ok(toml_str) = toml::to_string_pretty(&default_cfg) {
            let _ = fs::write(target_path, toml_str);
            info!("Создан шаблон конфигурации по умолчанию: {}", target_path.display());
        }

        Ok(default_cfg)
    }

    /// Формирование полного WebSocket URL для Sentinel Connect
    pub fn websocket_url(&self) -> Result<String> {
        let base = self.server_url.trim_end_matches('/');
        let ws_scheme = if base.starts_with("https://") {
            "wss://"
        } else if base.starts_with("http://") {
            "ws://"
        } else {
            "ws://"
        };

        let host = base
            .trim_start_matches("https://")
            .trim_start_matches("http://");

        let encoded_token = url::form_urlencoded::byte_serialize(self.auth_token.as_bytes()).collect::<String>();
        let encoded_device_id = url::form_urlencoded::byte_serialize(self.device_id.as_bytes()).collect::<String>();
        let encoded_name = url::form_urlencoded::byte_serialize(self.device_name.as_bytes()).collect::<String>();
        let encoded_type = url::form_urlencoded::byte_serialize(self.device_type.as_bytes()).collect::<String>();

        Ok(format!(
            "{}{}/api/remote/ws?token={}&device_id={}&device_name={}&device_type={}",
            ws_scheme, host, encoded_token, encoded_device_id, encoded_name, encoded_type
        ))
    }

    /// Формирование полного HTTP URL для аудиопотока трека
    pub fn stream_url(&self, track_id: i64) -> String {
        format!("{}/api/stream/{}", self.server_url.trim_end_matches('/'), track_id)
    }
}
