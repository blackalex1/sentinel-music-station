use std::sync::Arc;
use std::time::Duration;
use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};

use crate::config::StationConfig;
use crate::connect::protocol::{IncomingMessage, OutgoingMessage, PlayerCommand, PlayerState};

pub struct ConnectClient {
    config: Arc<StationConfig>,
    command_tx: mpsc::Sender<PlayerCommand>,
    state_rx: watch::Receiver<PlayerState>,
    is_connected_tx: watch::Sender<bool>,
    remote_state_tx: Option<watch::Sender<PlayerState>>,
}

impl ConnectClient {
    pub fn new(
        config: Arc<StationConfig>,
        command_tx: mpsc::Sender<PlayerCommand>,
        state_rx: watch::Receiver<PlayerState>,
        is_connected_tx: watch::Sender<bool>,
    ) -> Self {
        Self {
            config,
            command_tx,
            state_rx,
            is_connected_tx,
            remote_state_tx: None,
        }
    }

    pub fn with_remote_state_tx(mut self, tx: watch::Sender<PlayerState>) -> Self {
        self.remote_state_tx = Some(tx);
        self
    }

    /// Запуск супервайзера постоянного подключения к Sentinel Connect с авто-реконнектом
    pub async fn run_supervisor(&mut self) {
        let mut retry_delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(30);

        loop {
            info!("Подключение к Sentinel Connect по адресу: {}", self.config.server_url);
            match self.connect_and_process().await {
                Ok(_) => {
                    info!("Соединение с Sentinel Connect закрыто штатно.");
                    retry_delay = Duration::from_secs(1);
                }
                Err(e) => {
                    error!("Ошибка соединения Sentinel Connect: {:#}", e);
                }
            }

            let _ = self.is_connected_tx.send(false);

            if !self.config.auto_reconnect {
                info!("Авто-реконнект отключен в конфигурации. Завершение работы клиента.");
                break;
            }

            warn!("Повторная попытка подключения через {} сек...", retry_delay.as_secs());
            sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(max_delay);
        }
    }

    async fn connect_and_process(&mut self) -> Result<()> {
        let ws_url = self.config.websocket_url()?;
        let (ws_stream, response) = connect_async(&ws_url)
            .await
            .with_context(|| format!("Не удалось установить WebSocket соединение с {}", ws_url))?;

        info!("Успешное подключение к серверу! Код ответа HTTP: {}", response.status());
        let _ = self.is_connected_tx.send(true);

        let (mut write, mut read) = ws_stream.split();
        let ping_interval = Duration::from_secs(self.config.ping_interval_secs);
        let mut ping_ticker = tokio::time::interval(ping_interval);

        // Отправка начального состояния
        let initial_state = self.state_rx.borrow().clone();
        let initial_msg = serde_json::to_string(&OutgoingMessage::StateUpdate { state: initial_state })?;
        write.send(Message::Text(initial_msg)).await?;

        let mut local_state_rx = self.state_rx.clone();

        loop {
            tokio::select! {
                // Пинг-таймер для поддержки живости соединения
                _ = ping_ticker.tick() => {
                    let ping_msg = serde_json::to_string(&OutgoingMessage::Ping)?;
                    if let Err(e) = write.send(Message::Text(ping_msg)).await {
                        warn!("Ошибка отправки пинга: {}", e);
                        break;
                    }
                }

                // Изменение состояния плеера на колонке -> отправка серверу
                res = local_state_rx.changed() => {
                    if res.is_ok() {
                        let state = local_state_rx.borrow().clone();
                        let msg = OutgoingMessage::StateUpdate { state };
                        if let Ok(json_str) = serde_json::to_string(&msg) {
                            if let Err(e) = write.send(Message::Text(json_str)).await {
                                warn!("Ошибка отправки обновления состояния: {}", e);
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                }

                // Получение входящих команд от сервера
                ws_msg = read.next() => {
                    match ws_msg {
                        Some(Ok(Message::Text(text))) => {
                            self.handle_incoming_text(&text).await;
                        }
                        Some(Ok(Message::Ping(payload))) => {
                            let _ = write.send(Message::Pong(payload)).await;
                        }
                        Some(Ok(Message::Close(_))) => {
                            info!("Сервер отправил команду закрытия соединения (Close frame)");
                            break;
                        }
                        Some(Err(e)) => {
                            warn!("Ошибка сокета при чтении: {}", e);
                            break;
                        }
                        None => {
                            info!("Поток WebSocket завершен (EOF)");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_incoming_text(&self, text: &str) {
        match serde_json::from_str::<IncomingMessage>(text) {
            Ok(IncomingMessage::Welcome { device_id, devices }) => {
                info!("Добро пожаловать в Sentinel Connect! Зарегистрирован как ID: {}, активных устройств в сети: {}", device_id, devices.len());
                if let Some(ref tx) = self.remote_state_tx {
                    for dev in &devices {
                        let dev_id = dev.get("id").or_else(|| dev.get("device_id")).and_then(|v| v.as_str());
                        if dev_id != Some(&device_id) {
                            let state_val = dev.get("state").unwrap_or(dev);
                            if let Ok(st) = serde_json::from_value::<PlayerState>(state_val.clone()) {
                                info!("TUI: синхронизировано состояние с '{}' (играет: {}, громкость: {}%)", dev_id.unwrap_or("?"), st.is_playing, (st.volume * 100.0) as u32);
                                let _ = tx.send(st);
                                break;
                            }
                        }
                    }
                }
            }
            Ok(IncomingMessage::Command { from_device_id, command, payload }) => {
                let from = from_device_id.as_deref().unwrap_or("unknown");
                info!("Получена команда от '{}': [{}] (payload: {})", from, command, payload);
                if let Some(cmd) = self.parse_player_command(&command, &payload) {
                    if let Err(e) = self.command_tx.send(cmd).await {
                        error!("Не удалось доставить команду в аудиоплеер: {}", e);
                    }
                }
            }
            Ok(IncomingMessage::Pong { .. }) => {
                // Успешный ответ на пинг
            }
            Ok(IncomingMessage::Error { message }) => {
                error!("Сообщение об ошибке от сервера: {}", message);
            }
            Ok(IncomingMessage::DevicesUpdate { devices }) => {
                info!("Обновление списка устройств в доме: {} шт.", devices.len());
                if let Some(ref tx) = self.remote_state_tx {
                    for dev in &devices {
                        let dev_id = dev.get("id").or_else(|| dev.get("device_id")).and_then(|v| v.as_str());
                        if dev_id != Some(&self.config.device_id) {
                            let state_val = dev.get("state").unwrap_or(dev);
                            if let Ok(st) = serde_json::from_value::<PlayerState>(state_val.clone()) {
                                let _ = tx.send(st);
                                break;
                            }
                        }
                    }
                }
            }
            Ok(IncomingMessage::DeviceState { device_id, device }) => {
                tracing::debug!("Устройство {} обновило состояние", device_id);
                if let Some(ref tx) = self.remote_state_tx {
                    let state_val = device.get("state").unwrap_or(&device);
                    match serde_json::from_value::<PlayerState>(state_val.clone()) {
                        Ok(st) => {
                            let _ = tx.send(st);
                        }
                        Err(e) => {
                            tracing::warn!("Не удалось распарсить PlayerState из device_state от {}: {}", device_id, e);
                        }
                    }
                }
            }
            Ok(IncomingMessage::Unknown) => {
                tracing::debug!("Получено неизвестное сообщение: {}", text);
            }
            Err(e) => {
                warn!("Не удалось разобрать входящее сообщение: {}. Ошибка: {}", text, e);
            }
        }
    }

    fn parse_player_command(&self, cmd: &str, payload: &serde_json::Value) -> Option<PlayerCommand> {
        let cmd_upper = cmd.to_uppercase();
        match cmd_upper.as_str() {
            "PLAY" => Some(PlayerCommand::Play),
            "PAUSE" => Some(PlayerCommand::Pause),
            "TOGGLE_PLAY" | "TOGGLE" => Some(PlayerCommand::TogglePlay),
            "STOP" => Some(PlayerCommand::Stop),
            "NEXT" => Some(PlayerCommand::Next),
            "PREV" => Some(PlayerCommand::Prev),
            "SEEK" => {
                let pos_ms = payload.get("position_ms")
                    .and_then(|v| {
                        v.as_u64()
                            .or_else(|| v.as_i64().map(|i| i.max(0) as u64))
                            .or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))
                            .or_else(|| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                    })
                    .or_else(|| {
                        payload.get("position_seconds")
                            .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
                            .map(|s| (s.max(0.0) * 1000.0) as u64)
                    })
                    .or_else(|| {
                        payload.get("position")
                            .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|u| u as f64)))
                            .map(|s| (s.max(0.0) * 1000.0) as u64)
                    })
                    .unwrap_or(0);
                Some(PlayerCommand::Seek { position_ms: pos_ms })
            }
            "SET_VOLUME" | "VOLUME" => {
                let vol = payload.get("volume")
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32)
                    .unwrap_or(0.7);
                let normalized = if vol > 1.0 { vol / 100.0 } else { vol };
                Some(PlayerCommand::SetVolume { volume: normalized })
            }
            "SHUFFLE" | "TOGGLE_SHUFFLE" => {
                if let Some(enabled) = payload.get("enabled").and_then(|v| v.as_bool())
                    .or_else(|| payload.get("is_shuffle").and_then(|v| v.as_bool()))
                {
                    Some(PlayerCommand::SetShuffle { enabled })
                } else {
                    Some(PlayerCommand::ToggleShuffle)
                }
            }
            "REPEAT" | "SET_REPEAT" => {
                let mode = payload.get("mode")
                    .and_then(|v| v.as_str())
                    .or_else(|| payload.get("repeat_mode").and_then(|v| v.as_str()))
                    .unwrap_or("off")
                    .to_string();
                Some(PlayerCommand::SetRepeat { mode })
            }
            "PLAY_TRACK" | "PREPARE_TRACK" | "TRANSFER_PLAYBACK" => {
                let track_obj = payload.get("track");
                let queue: Vec<crate::connect::protocol::TrackMeta> = payload.get("queue")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();

                let first_queue_track = queue.first();

                let track_id = payload.get("track_id")
                    .and_then(|v| v.as_i64())
                    .or_else(|| payload.get("id").and_then(|v| v.as_i64()))
                    .or_else(|| track_obj.and_then(|t| t.get("id")).and_then(|v| v.as_i64()))
                    .or_else(|| first_queue_track.map(|t| t.id))?;

                let title = payload.get("title")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .or_else(|| track_obj.and_then(|t| t.get("title")).and_then(|v| v.as_str().map(|s| s.to_string())))
                    .or_else(|| first_queue_track.map(|t| t.title.clone()))
                    .unwrap_or_else(|| "Неизвестный трек".to_string());

                let artist = payload.get("artist")
                    .and_then(|v| v.as_str().map(|s| s.to_string()))
                    .or_else(|| track_obj.and_then(|t| t.get("artist")).and_then(|v| v.as_str().map(|s| s.to_string())))
                    .or_else(|| first_queue_track.map(|t| t.artist.clone()))
                    .unwrap_or_else(|| "Неизвестный исполнитель".to_string());

                let duration_ms = payload.get("duration_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| payload.get("duration").and_then(|v| v.as_f64()).map(|f| (f * 1000.0) as u64))
                    .or_else(|| track_obj.and_then(|t| t.get("duration_ms")).and_then(|v| v.as_u64()))
                    .or_else(|| track_obj.and_then(|t| t.get("duration")).and_then(|v| v.as_f64()).map(|f| (f * 1000.0) as u64))
                    .or_else(|| first_queue_track.and_then(|t| t.duration).map(|d| (d * 1000.0) as u64))
                    .unwrap_or(0);

                let position_ms = payload.get("position_ms")
                    .and_then(|v| v.as_u64())
                    .or_else(|| payload.get("position_seconds").and_then(|v| v.as_f64()).map(|f| (f * 1000.0) as u64))
                    .unwrap_or(0);

                let auto_play = payload.get("auto_play")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                Some(PlayerCommand::PlayTrack {
                    track_id,
                    title,
                    artist,
                    duration_ms,
                    position_ms,
                    auto_play,
                    queue,
                })
            }
            _ => {
                warn!("Неизвестная команда воспроизведения: {}", cmd);
                None
            }
        }
    }
}
