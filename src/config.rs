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

    /// Стиль анимации переключения треков: "random", "curtain", "tape", "slot", "chevron"
    #[serde(default = "default_track_animation_style")]
    pub track_animation_style: String,

    /// Режим работы эквалайзера спектра при воспроизведении музыки:
    /// "always" (по умолчанию) — показывать спектр постоянно при игре музыки,
    /// "intro" — показывать спектр 10 секунд при старте трека, затем часы,
    /// "off" — всегда отображать часы, спектр отключен
    #[serde(default = "default_equalizer_mode")]
    pub equalizer_mode: String,

    /// Уровень программной громкости музыки при активности Алисы (Audio Ducking, от 0.0 до 1.0, по умолчанию 0.15)
    #[serde(default = "default_ducking_volume")]
    pub ducking_volume: f32,

    /// Воспроизводить голосовое приветствие при старте демона (по умолчанию false — тихий запуск)
    #[serde(default = "default_false")]
    pub startup_greeting: bool,

    /// Разрешать подключение к серверу с недоверенным/самоподписанным SSL сертификатом
    #[serde(default = "default_false", alias = "insecure_ssl")]
    pub allow_insecure_ssl: bool,
}

fn default_ducking_volume() -> f32 {
    0.15
}

fn default_equalizer_mode() -> String {
    "always".to_string()
}

fn default_false() -> bool {
    false
}

fn default_track_animation_style() -> String {
    "random".to_string()
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
            track_animation_style: default_track_animation_style(),
            equalizer_mode: default_equalizer_mode(),
            ducking_volume: default_ducking_volume(),
            startup_greeting: false,
            allow_insecure_ssl: false,
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
        let mut u = url::Url::parse(&self.server_url)
            .map_err(|e| anyhow::anyhow!("Неверный server_url: {}", e))?;

        let scheme = match u.scheme() {
            "https" | "wss" => "wss",
            _ => "ws",
        };
        u.set_scheme(scheme)
            .map_err(|_| anyhow::anyhow!("Не удалось установить схему {}", scheme))?;

        u.set_path("/api/remote/ws");
        u.query_pairs_mut()
            .clear()
            .append_pair("token", &self.auth_token)
            .append_pair("device_id", &self.device_id)
            .append_pair("device_name", &self.device_name)
            .append_pair("device_type", &self.device_type);

        Ok(u.to_string())
    }

    /// Формирование полного HTTP URL для аудиопотока трека
    pub fn stream_url(&self, track_id: i64) -> String {
        format!("{}/api/stream/{}", self.server_url.trim_end_matches('/'), track_id)
    }
}
