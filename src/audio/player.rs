use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use rodio::{Decoder, OutputStreamHandle, Sink, Source};
use tokio::sync::{mpsc, watch};
use tracing::{error, info};

use crate::audio::loader::TrackLoader;
use crate::audio::queue::PlaybackQueue;
use crate::audio::routing::{set_hardware_volume, sync_hardware_routing};
use crate::audio::source::BufferedStreamSource;
use crate::config::StationConfig;
use crate::connect::protocol::{PlayerCommand, PlayerState, TrackMeta};

/// Главный контроллер аудиовоспроизведения Станции
pub struct AudioPlayer {
    config: Arc<StationConfig>,
    command_rx: mpsc::Receiver<PlayerCommand>,
    state_tx: watch::Sender<PlayerState>,
    sample_buffer: Arc<Mutex<Vec<f32>>>,
    loader: TrackLoader,
    queue: PlaybackQueue,
    stream_handle: OutputStreamHandle,
    sink: Arc<Mutex<Sink>>,
    jack_state: Option<bool>,
    current_audio_data: Option<bytes::Bytes>,
    current_track_id: Option<i64>,
}

impl AudioPlayer {
    pub fn new(
        config: Arc<StationConfig>,
        stream_handle: OutputStreamHandle,
        command_rx: mpsc::Receiver<PlayerCommand>,
        state_tx: watch::Sender<PlayerState>,
        sample_buffer: Arc<Mutex<Vec<f32>>>,
    ) -> Result<Self> {
        let sink = Sink::try_new(&stream_handle)
            .context("Не удалось создать аудиоплеер Sink")?;

        sink.set_volume(1.0);

        let mut jack_state = None;
        sync_hardware_routing(&mut jack_state, &config.audio_routing);
        set_hardware_volume(config.initial_volume);

        let loader = TrackLoader::new(config.clone())?;

        Ok(Self {
            config,
            command_rx,
            state_tx,
            sample_buffer,
            loader,
            queue: PlaybackQueue::new(),
            stream_handle,
            sink: Arc::new(Mutex::new(sink)),
            jack_state,
            current_audio_data: None,
            current_track_id: None,
        })
    }

    /// Главный цикл обработки команд воспроизведения и отслеживания прогресса
    pub async fn run_loop(&mut self) {
        let mut progress_ticker = tokio::time::interval(Duration::from_millis(500));
        let mut track_started_at: Option<Instant> = None;
        let mut accumulated_position = Duration::ZERO;
        let mut jack_check_counter = 0u32;

        loop {
            tokio::select! {
                // Обработка команд управления (Play, Pause, Seek, Volume, Queue)
                Some(cmd) = self.command_rx.recv() => {
                    match cmd {
                        PlayerCommand::PlayTrack { track_id, title, artist, duration_ms, position_ms, auto_play, queue } => {
                            info!("Запуск трека ID: {} - '{}' / '{}' (позиция: {} мс, auto_play: {})", track_id, title, artist, position_ms, auto_play);
                            self.queue.set_queue(queue, track_id);

                            match self.load_and_play_track_at(track_id, position_ms, auto_play).await {
                                Ok(()) => {
                                    track_started_at = if auto_play { Some(Instant::now()) } else { None };
                                    accumulated_position = Duration::from_millis(position_ms);

                                    let mut state = self.state_tx.borrow().clone();
                                    state.is_playing = auto_play;
                                    state.position_ms = position_ms;
                                    state.duration_ms = duration_ms;
                                    state.queue = self.queue.tracks();
                                    state.current_track = Some(TrackMeta {
                                        id: track_id,
                                        title,
                                        artist,
                                        album: None,
                                        duration: Some(duration_ms as f64 / 1000.0),
                                        cover_url: Some(format!("{}/api/covers/{}", self.config.server_url.trim_end_matches('/'), track_id)),
                                    });
                                    let _ = self.state_tx.send(state);
                                }
                                Err(e) => {
                                    error!("Ошибка воспроизведения трека {}: {:#}", track_id, e);
                                }
                            }
                        }
                        PlayerCommand::Play => {
                            sync_hardware_routing(&mut self.jack_state, &self.config.audio_routing);
                            let is_empty = self.sink.lock().map(|s| s.empty()).unwrap_or(false);
                            if is_empty {
                                if self.queue.is_empty() {
                                    if let Ok(tracks) = self.loader.fetch_server_tracks().await {
                                        self.queue.set_queue(tracks, 0);
                                    }
                                }
                                if let Some(track) = self.queue.current().cloned() {
                                    let dur_ms = track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                    if let Ok(()) = self.load_and_play_track(track.id).await {
                                        track_started_at = Some(Instant::now());
                                        accumulated_position = Duration::ZERO;
                                        let mut state = self.state_tx.borrow().clone();
                                        state.is_playing = true;
                                        state.position_ms = 0;
                                        state.duration_ms = dur_ms;
                                        state.current_track = Some(track);
                                        state.queue = self.queue.tracks();
                                        let _ = self.state_tx.send(state);
                                    }
                                }
                            } else {
                                if let Ok(sink) = self.sink.lock() {
                                    sink.play();
                                }
                                if track_started_at.is_none() {
                                    track_started_at = Some(Instant::now());
                                }
                                let mut state = self.state_tx.borrow().clone();
                                state.is_playing = true;
                                let _ = self.state_tx.send(state);
                            }
                        }
                        PlayerCommand::Pause => {
                            if let Ok(sink) = self.sink.lock() {
                                sink.pause();
                            }
                            if let Some(started) = track_started_at.take() {
                                accumulated_position += started.elapsed();
                            }
                            let mut state = self.state_tx.borrow().clone();
                            state.is_playing = false;
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::TogglePlay => {
                            sync_hardware_routing(&mut self.jack_state, &self.config.audio_routing);
                            let mut is_playing = false;
                            let mut is_empty = false;
                            if let Ok(sink) = self.sink.lock() {
                                if sink.empty() {
                                    is_empty = true;
                                } else if sink.is_paused() {
                                    sink.play();
                                    is_playing = true;
                                } else {
                                    sink.pause();
                                }
                            }
                            if is_empty {
                                // Если ничего не играло, запускаем трек из очереди или подтягиваем с сервера
                                if self.queue.is_empty() {
                                    if let Ok(tracks) = self.loader.fetch_server_tracks().await {
                                        self.queue.set_queue(tracks, 0);
                                    }
                                }
                                if let Some(track) = self.queue.current().cloned() {
                                    let dur_ms = track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                    if let Ok(()) = self.load_and_play_track(track.id).await {
                                        track_started_at = Some(Instant::now());
                                        accumulated_position = Duration::ZERO;
                                        let mut state = self.state_tx.borrow().clone();
                                        state.is_playing = true;
                                        state.position_ms = 0;
                                        state.duration_ms = dur_ms;
                                        state.current_track = Some(track);
                                        state.queue = self.queue.tracks();
                                        let _ = self.state_tx.send(state);
                                    }
                                }
                            } else {
                                if is_playing {
                                    track_started_at = Some(Instant::now());
                                } else if let Some(started) = track_started_at.take() {
                                    accumulated_position += started.elapsed();
                                }
                                let mut state = self.state_tx.borrow().clone();
                                state.is_playing = is_playing;
                                let _ = self.state_tx.send(state);
                            }
                        }
                        PlayerCommand::Stop => {
                            if let Ok(sink) = self.sink.lock() {
                                sink.stop();
                            }
                            track_started_at = None;
                            accumulated_position = Duration::ZERO;
                            let mut state = self.state_tx.borrow().clone();
                            state.is_playing = false;
                            state.position_ms = 0;
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::Seek { position_ms } => {
                            info!("Получена команда перемотки (SEEK) на {} мс ({:.1} сек)", position_ms, position_ms as f64 / 1000.0);
                            let was_playing = if let Ok(sink) = self.sink.lock() {
                                !sink.is_paused() && !sink.empty()
                            } else {
                                false
                            };

                            let cur_track_id = self.state_tx.borrow().current_track.as_ref().map(|t| t.id);
                            let audio_opt = self.current_audio_data.clone();
                            let seek_res = if let Some(audio_bytes) = audio_opt {
                                self.seek_and_play(audio_bytes, position_ms, was_playing)
                            } else if let Some(track_id) = cur_track_id {
                                if let Ok(()) = self.load_and_play_track(track_id).await {
                                    if let Some(bytes) = self.current_audio_data.clone() {
                                        self.seek_and_play(bytes, position_ms, was_playing)
                                    } else {
                                        Err(anyhow::anyhow!("Буфер аудиоданных недоступен"))
                                    }
                                } else {
                                    Err(anyhow::anyhow!("Не удалось загрузить трек для перемотки"))
                                }
                            } else {
                                Err(anyhow::anyhow!("Нет активного трека для перемотки"))
                            };

                            match seek_res {
                                Ok(()) => {
                                    accumulated_position = Duration::from_millis(position_ms);
                                    track_started_at = if was_playing { Some(Instant::now()) } else { None };

                                    let mut state = self.state_tx.borrow().clone();
                                    state.position_ms = position_ms;
                                    state.is_playing = was_playing;
                                    let _ = self.state_tx.send(state);
                                    info!("Перемотка на {:.1} сек успешно выполнена", position_ms as f64 / 1000.0);
                                }
                                Err(e) => {
                                    error!("Ошибка при выполнении перемотки: {:#}", e);
                                }
                            }
                        }
                        PlayerCommand::SetVolume { volume } => {
                            let clamped = volume.clamp(0.0, 1.0);
                            info!("🔊 Установка громкости: {:.0}%", clamped * 100.0);
                            if let Ok(sink) = self.sink.lock() {
                                sink.set_volume(1.0);
                            }
                            set_hardware_volume(clamped);
                            let mut state = self.state_tx.borrow().clone();
                            state.volume = clamped;
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::Next => {
                            info!("Команда переключения на следующий трек (Next)");
                            if self.queue.is_empty() {
                                if let Ok(tracks) = self.loader.fetch_server_tracks().await {
                                    self.queue.set_queue(tracks, 0);
                                }
                            }
                            if let Some(next_track) = self.queue.next() {
                                let dur_ms = next_track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                if let Ok(()) = self.load_and_play_track(next_track.id).await {
                                    track_started_at = Some(Instant::now());
                                    accumulated_position = Duration::ZERO;
                                    let mut state = self.state_tx.borrow().clone();
                                    state.is_playing = true;
                                    state.position_ms = 0;
                                    state.duration_ms = dur_ms;
                                    state.current_track = Some(next_track);
                                    state.queue = self.queue.tracks();
                                    let _ = self.state_tx.send(state);
                                }
                            }
                        }
                        PlayerCommand::Prev => {
                            info!("Команда переключения на предыдущий трек (Prev)");
                            if self.queue.is_empty() {
                                if let Ok(tracks) = self.loader.fetch_server_tracks().await {
                                    self.queue.set_queue(tracks, 0);
                                }
                            }
                            if let Some(prev_track) = self.queue.prev() {
                                let dur_ms = prev_track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                if let Ok(()) = self.load_and_play_track(prev_track.id).await {
                                    track_started_at = Some(Instant::now());
                                    accumulated_position = Duration::ZERO;
                                    let mut state = self.state_tx.borrow().clone();
                                    state.is_playing = true;
                                    state.position_ms = 0;
                                    state.duration_ms = dur_ms;
                                    state.current_track = Some(prev_track);
                                    state.queue = self.queue.tracks();
                                    let _ = self.state_tx.send(state);
                                }
                            }
                        }
                        PlayerCommand::ToggleShuffle => {
                            let new_shuffle = self.queue.toggle_shuffle();
                            let mut state = self.state_tx.borrow().clone();
                            state.is_shuffle = new_shuffle;
                            info!("🔀 Режим перемешивания (Shuffle): {}", new_shuffle);
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::SetShuffle { enabled } => {
                            self.queue.set_shuffle(enabled);
                            let mut state = self.state_tx.borrow().clone();
                            state.is_shuffle = enabled;
                            info!("🔀 Установлен режим перемешивания (Shuffle): {}", enabled);
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::SetRepeat { mode } => {
                            let mut state = self.state_tx.borrow().clone();
                            info!("🔁 Установлен режим повтора (Repeat): {}", mode);
                            state.repeat_mode = mode;
                            let _ = self.state_tx.send(state);
                        }
                    }
                }

                // Таймер обновления текущей позиции воспроизведения, автоперехода и детекции AUX Jack
                _ = progress_ticker.tick() => {
                    // Периодическая проверка подключения кабеля 3.5мм (раз в 1.5 секунды)
                    jack_check_counter = (jack_check_counter + 1) % 3;
                    if jack_check_counter == 0 {
                        sync_hardware_routing(&mut self.jack_state, &self.config.audio_routing);
                    }

                    let is_empty;
                    let is_paused;
                    if let Ok(sink) = self.sink.lock() {
                        is_empty = sink.empty();
                        is_paused = sink.is_paused();
                    } else {
                        continue;
                    }

                    // Защита от ложного срабатывания empty() в первые 2 секунды после запуска трека
                    let just_started = track_started_at
                        .map(|t| t.elapsed() < Duration::from_secs(2))
                        .unwrap_or(false);

                    if is_empty && !just_started {
                        let mut state = self.state_tx.borrow().clone();
                        if state.is_playing {
                            let repeat = state.repeat_mode.as_str();
                            if repeat == "one" || repeat == "track" {
                                if let Some(cur) = state.current_track.clone() {
                                    info!("🔁 Повтор текущего трека: '{}'", cur.title);
                                    let dur_ms = cur.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                    if let Ok(()) = self.load_and_play_track(cur.id).await {
                                        track_started_at = Some(Instant::now());
                                        accumulated_position = Duration::ZERO;
                                        state.is_playing = true;
                                        state.position_ms = 0;
                                        state.duration_ms = dur_ms;
                                        let _ = self.state_tx.send(state);
                                        continue;
                                    }
                                }
                            }

                            // Трек завершился: пробуем автоматически перейти к следующему треку в очереди
                            if let Some(next_track) = self.queue.next() {
                                info!("Автовоспроизведение следующего трека: '{}'", next_track.title);
                                let dur_ms = next_track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                if let Ok(()) = self.load_and_play_track(next_track.id).await {
                                    track_started_at = Some(Instant::now());
                                    accumulated_position = Duration::ZERO;
                                    state.is_playing = true;
                                    state.position_ms = 0;
                                    state.duration_ms = dur_ms;
                                    state.current_track = Some(next_track);
                                    let _ = self.state_tx.send(state);
                                    continue;
                                }
                            }

                            state.is_playing = false;
                            track_started_at = None;
                            accumulated_position = Duration::ZERO;
                            let _ = self.state_tx.send(state);
                        }
                    } else if !is_paused {
                        let current_pos = accumulated_position + track_started_at.map_or(Duration::ZERO, |t| t.elapsed());
                        let mut state = self.state_tx.borrow().clone();
                        state.position_ms = current_pos.as_millis() as u64;
                        let _ = self.state_tx.send(state);
                    }
                }
            }
        }
    }

    /// Загрузка аудиоданных по сети и добавление в Sink плеера с указанной позиции и статусом auto_play
    async fn load_and_play_track_at(&mut self, track_id: i64, position_ms: u64, auto_play: bool) -> Result<()> {
        sync_hardware_routing(&mut self.jack_state, &self.config.audio_routing);

        let audio_bytes = self.loader.download_stream(track_id).await?;
        self.current_audio_data = Some(audio_bytes.clone());
        self.current_track_id = Some(track_id);

        if position_ms > 0 {
            self.seek_and_play(audio_bytes, position_ms, auto_play)
        } else {
            let cursor = Cursor::new(audio_bytes);
            let decoder = Decoder::new(cursor)
                .context("Не удалось декодировать аудиопоток (поддерживаются FLAC, MP3, AAC, WAV)")?;

            // Очищаем буфер визуализатора перед началом нового трека
            if let Ok(mut buf) = self.sample_buffer.lock() {
                buf.clear();
            }

            // Оборачиваем аудиоисточник в предзагруженный потоковый буфер с защитой от XRUN/underrun
            let buffered_source = BufferedStreamSource::new(
                decoder.convert_samples::<f32>(),
                self.sample_buffer.clone(),
                self.config.visualizer_enabled,
            );

            let sink_lock = self.sink.lock();
            if let Ok(mut sink) = sink_lock {
                // Обязательно останавливаем старый Sink, чтобы rodio удалил его источники из микшера
                sink.stop();
                let new_sink = Sink::try_new(&self.stream_handle)
                    .context("Не удалось пересоздать Sink")?;
                new_sink.set_volume(1.0);
                new_sink.append(buffered_source);
                if auto_play {
                    new_sink.play();
                } else {
                    new_sink.pause();
                }
                *sink = new_sink;
            }

            Ok(())
        }
    }

    /// Загрузка аудиоданных по сети и немедленное воспроизведение с начала
    async fn load_and_play_track(&mut self, track_id: i64) -> Result<()> {
        self.load_and_play_track_at(track_id, 0, true).await
    }

    /// Перемотка трека на заданную позицию без повторного скачивания по сети
    fn seek_and_play(&mut self, audio_bytes: bytes::Bytes, position_ms: u64, auto_play: bool) -> Result<()> {
        sync_hardware_routing(&mut self.jack_state, &self.config.audio_routing);

        let cursor = Cursor::new(audio_bytes);
        let mut decoder = Decoder::new(cursor)
            .context("Не удалось декодировать аудиопоток для перемотки")?;

        let seek_duration = Duration::from_millis(position_ms);

        // 1. Пробуем быстрый нативный try_seek через контейнер / Symphonia
        if let Err(e) = decoder.try_seek(seek_duration) {
            tracing::debug!("Decoder::try_seek вернул ({:?}), выполняем перемотку пропуском сэмплов...", e);
            let sample_rate = decoder.sample_rate();
            let channels = decoder.channels();
            let samples_to_skip = (position_ms as u64 * sample_rate as u64 * channels as u64) / 1000;
            for _ in 0..samples_to_skip {
                if decoder.next().is_none() {
                    break;
                }
            }
        }

        // 2. Очищаем буфер спектроанализатора
        if let Ok(mut buf) = self.sample_buffer.lock() {
            buf.clear();
        }

        // 3. Создаем новый буферизованный источник с позиции перемотки
        let buffered_source = BufferedStreamSource::new(
            decoder.convert_samples::<f32>(),
            self.sample_buffer.clone(),
            self.config.visualizer_enabled,
        );

        // 4. Пересоздаем Sink и применяем громкость и статус паузы
        let sink_lock = self.sink.lock();
        if let Ok(mut sink) = sink_lock {
            sink.stop();
            let new_sink = Sink::try_new(&self.stream_handle)
                .context("Не удалось пересоздать Sink для перемотки")?;
            new_sink.set_volume(1.0);
            new_sink.append(buffered_source);
            if auto_play {
                new_sink.play();
            } else {
                new_sink.pause();
            }
            *sink = new_sink;
        }

        Ok(())
    }
}
