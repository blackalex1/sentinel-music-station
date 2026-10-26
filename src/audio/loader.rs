use std::sync::Arc;
use std::time::Duration;
use anyhow::{Context, Result};
use tracing::info;

use crate::config::StationConfig;
use crate::connect::protocol::TrackMeta;

/// Загрузчик аудиопотоков и метаданных треков с сервера Sentinel Music
pub struct TrackLoader {
    config: Arc<StationConfig>,
    http_client: reqwest::Client,
}

impl TrackLoader {
    pub fn new(config: Arc<StationConfig>) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .build()?;

        Ok(Self {
            config,
            http_client,
        })
    }

    /// Загрузка бинарных аудиоданных трека по HTTP
    pub async fn download_stream(&self, track_id: i64) -> Result<bytes::Bytes> {
        let stream_url = self.config.stream_url(track_id);
        info!("Загрузка аудиопотока с: {}", stream_url);

        let mut request = self.http_client.get(&stream_url);
        if !self.config.auth_token.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.config.auth_token));
        }

        let response = request.send().await
            .with_context(|| format!("Не удалось отправить запрос к {}", stream_url))?;

        if !response.status().is_success() {
            anyhow::bail!("Сервер вернул ошибку при запросе аудиопотока: HTTP {}", response.status());
        }

        let audio_bytes = response.bytes().await
            .context("Ошибка чтения тела аудиопотока")?;

        info!("Загружено {} байт аудио. Декодирование...", audio_bytes.len());
        Ok(audio_bytes)
    }

    /// Запрос списка треков с сервера при отсутствии локальной очереди
    pub async fn fetch_server_tracks(&self) -> Result<Vec<TrackMeta>> {
        let tracks_url = format!("{}/api/tracks?limit=50", self.config.server_url.trim_end_matches('/'));
        let mut request = self.http_client.get(&tracks_url);
        if !self.config.auth_token.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.config.auth_token));
        }

        let response = request.send().await
            .with_context(|| format!("Не удалось запросить список треков с {}", tracks_url))?;

        if response.status().is_success() {
            let tracks: Vec<TrackMeta> = response.json().await
                .context("Ошибка разбора списка треков от сервера")?;
            Ok(tracks)
        } else {
            anyhow::bail!("Сервер вернул статус HTTP {} при получении списка треков", response.status());
        }
    }

    /// Запрос следующего рекомендованного трека для бесконечного режима Поток (Flow)
    pub async fn fetch_next_stream_track(
        &self,
        current_track_id: Option<i64>,
        recent_ids: &[i64],
        mood: &str,
    ) -> Result<TrackMeta> {
        let mut url = format!(
            "{}/api/stream/next?mood={}&discovery_ratio=0.2",
            self.config.server_url.trim_end_matches('/'),
            mood
        );
        if let Some(id) = current_track_id {
            url.push_str(&format!("&current_track_id={}", id));
        }
        if !recent_ids.is_empty() {
            let ids_str = recent_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>().join(",");
            url.push_str(&format!("&recent_ids={}", ids_str));
        }

        info!("Запрос трека Потока с: {}", url);
        let mut request = self.http_client.get(&url);
        if !self.config.auth_token.is_empty() {
            request = request.header("Authorization", format!("Bearer {}", self.config.auth_token));
        }

        let response = request.send().await
            .with_context(|| format!("Не удалось отправить запрос к {}", url))?;

        if !response.status().is_success() {
            anyhow::bail!("Сервер вернул статус HTTP {} при получении трека Потока", response.status());
        }

        let track: TrackMeta = response.json().await
            .context("Ошибка разбора JSON трека Потока от сервера")?;

        Ok(track)
    }
}
