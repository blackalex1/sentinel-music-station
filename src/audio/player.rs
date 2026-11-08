use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use tokio::sync::{mpsc, watch};
use tracing::{error, info};

use crate::audio::loader::TrackLoader;
use crate::audio::queue::PlaybackQueue;
use crate::audio::routing::{load_persisted_volume, save_persisted_volume, set_hardware_volume, sync_hardware_routing};
use crate::audio::source::BufferedStreamSource;
use crate::config::StationConfig;
use crate::connect::protocol::{PlayerCommand, PlayerState, TrackMeta};

/// Обертка над rodio::OutputStream с маркером Send для переноса между асинхронными задачами Tokio
struct AudioOutputGuard(#[allow(dead_code)] OutputStream);
unsafe impl Send for AudioOutputGuard {}

/// Главный контроллер аудиовоспроизведения Станции
pub struct AudioPlayer {
    config: Arc<StationConfig>,
    command_rx: mpsc::Receiver<PlayerCommand>,
    state_tx: watch::Sender<PlayerState>,
    sample_buffer: Arc<Mutex<Vec<f32>>>,
    loader: TrackLoader,
    queue: PlaybackQueue,
    audio_stream: Option<AudioOutputGuard>,
    stream_handle: Option<OutputStreamHandle>,
    sink: Option<Arc<Mutex<Sink>>>,
    jack_state: Option<bool>,
    current_audio_data: Option<bytes::Bytes>,
    current_track_id: Option<i64>,
    is_flow_active: bool,
    recent_flow_ids: Vec<i64>,
    is_ducked: bool,
    idle_since: Option<Instant>,
}

impl AudioPlayer {
    pub fn new(
        config: Arc<StationConfig>,
        command_rx: mpsc::Receiver<PlayerCommand>,
        state_tx: watch::Sender<PlayerState>,
        sample_buffer: Arc<Mutex<Vec<f32>>>,
    ) -> Result<Self> {
        let mut jack_state = None;
        sync_hardware_routing(&mut jack_state, &config.audio_routing);

        let init_vol = load_persisted_volume().unwrap_or(config.initial_volume);
        set_hardware_volume(init_vol);
        {
            let mut state = state_tx.borrow().clone();
            state.volume = init_vol;
            let _ = state_tx.send(state);
        }

        let loader = TrackLoader::new(config.clone())?;

        Ok(Self {
            config,
            command_rx,
            state_tx,
            sample_buffer,
            loader,
            queue: PlaybackQueue::new(),
            audio_stream: None,
            stream_handle: None,
            sink: None,
            jack_state,
            current_audio_data: None,
            current_track_id: None,
            is_flow_active: false,
            recent_flow_ids: Vec::new(),
            is_ducked: false,
            idle_since: Some(Instant::now()),
        })
    }

    /// Ленивая инициализация аудиоустройства только при фактическом воспроизведении
    fn ensure_audio_stream(&mut self) -> Result<OutputStreamHandle> {
        self.idle_since = None;
        if self.stream_handle.is_none() {
            info!("🎵 Открытие ALSA аудиоустройства для воспроизведения...");
            let (stream, handle) = OutputStream::try_default()
                .context("Не удалось открыть аудиоустройство ALSA/Audio")?;
            self.audio_stream = Some(AudioOutputGuard(stream));
            self.stream_handle = Some(handle);
        }
        Ok(self.stream_handle.as_ref().unwrap().clone())
    }

    /// Освобождение аудиоустройства в режим сна/простоя (0% CPU, уступает ALSA Алисе)
    fn release_audio(&mut self) {
        if self.audio_stream.is_some() || self.sink.is_some() {
            info!("💤 Освобождение ALSA аудиоустройства в режим сна/простоя (0% CPU)");
            if let Some(sink) = self.sink.take() {
                if let Ok(s) = sink.lock() {
                    s.stop();
                }
            }
            self.sink = None;
            self.stream_handle = None;
            self.audio_stream = None;
        }
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
                        PlayerCommand::PlayTrack {
                            track_id,
                            title,
                            artist,
                            duration_ms,
                            position_ms,
                            auto_play,
                            queue,
                            is_preview,
                            candidate_id,
                            cover_url,
                            is_liked,
                            is_disliked,
                        } => {
                            info!("Запуск трека ID: {} (preview: {}, like: {}) - '{}' / '{}' (позиция: {} мс, auto_play: {})", track_id, is_preview, is_liked, title, artist, position_ms, auto_play);
                            self.is_flow_active = false;
                            self.recent_flow_ids.clear();
                            self.queue.set_queue(queue, track_id);

                            // Если включен режим Shuffle и в очереди мало треков (например, запуск из поиска):
                            // сразу подтягиваем случайные треки со всего сервера
                            if self.queue.is_shuffle() && !is_preview && self.queue.remaining() < 5 {
                                if let Ok(shuffled) = self.loader.fetch_shuffled_tracks(50, &[track_id]).await {
                                    info!("🔀 [Shuffle] Очередь дополнена случайными треками со всего сервера (всего: {} треков)", shuffled.len());
                                    self.queue.append_tracks(shuffled);
                                }
                            }

                            // Мгновенное оповещение о выбранном треке в сеть и на LED-экран (до скачивания потока)
                            let resolved_cover = cover_url.or_else(|| if track_id > 0 {
                                Some(format!("{}/api/covers/{}", self.config.server_url.trim_end_matches('/'), track_id))
                            } else {
                                None
                            });

                            let track_meta = TrackMeta {
                                id: track_id,
                                title,
                                artist,
                                album: None,
                                duration: Some(duration_ms as f64 / 1000.0),
                                cover_url: resolved_cover,
                                is_preview,
                                candidate_id: candidate_id.clone(),
                                is_liked,
                                is_disliked,
                            };

                            let mut pre_state = self.state_tx.borrow().clone();
                            pre_state.is_playing = auto_play;
                            pre_state.is_flow = false;
                            pre_state.position_ms = position_ms;
                            pre_state.duration_ms = duration_ms;
                            pre_state.queue = self.queue.tracks();
                            pre_state.current_track = Some(track_meta);
                            let _ = self.state_tx.send(pre_state);

                            let stream_res = if is_preview && candidate_id.is_some() {
                                self.load_and_play_candidate_preview(candidate_id.as_deref().unwrap(), position_ms, auto_play).await
                            } else {
                                self.load_and_play_track_at(track_id, position_ms, auto_play).await
                            };

                            match stream_res {
                                Ok(()) => {
                                    track_started_at = if auto_play { Some(Instant::now()) } else { None };
                                    accumulated_position = Duration::from_millis(position_ms);
                                    let mut state = self.state_tx.borrow().clone();
                                    state.is_playing = auto_play;
                                    let _ = self.state_tx.send(state);
                                }
                                Err(e) => {
                                    error!("Ошибка воспроизведения трека {}: {:#}", track_id, e);
                                }
                            }
                        }
                        PlayerCommand::SetQueue { queue, track_id } => {
                            info!("Получена обновленная очередь воспроизведения ({} треков, активный ID: {})", queue.len(), track_id);
                            let cur_id = if track_id > 0 {
                                track_id
                            } else {
                                self.state_tx.borrow().current_track.as_ref().map(|t| t.id).unwrap_or(0)
                            };
                            self.queue.set_queue(queue, cur_id);
                            let mut state = self.state_tx.borrow().clone();
                            state.queue = self.queue.tracks();
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::SetReaction { track_id, is_liked, is_disliked } => {
                            info!("Обновление реакции для трека {}: like={}, dislike={}", track_id, is_liked, is_disliked);
                            self.queue.update_reaction(track_id, is_liked, is_disliked);
                            let mut state = self.state_tx.borrow().clone();
                            if let Some(ref mut cur) = state.current_track {
                                if track_id == 0 || cur.id == track_id {
                                    cur.is_liked = is_liked;
                                    cur.is_disliked = is_disliked;
                                }
                            }
                            state.queue = self.queue.tracks();
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::Play => {
                            sync_hardware_routing(&mut self.jack_state, &self.config.audio_routing);
                            let is_empty = self.sink.as_ref().and_then(|s| s.lock().ok()).map(|s| s.empty()).unwrap_or(true);
                            if is_empty {
                                if self.queue.is_empty() {
                                    if let Ok(tracks) = self.loader.fetch_server_tracks().await {
                                        self.queue.set_queue(tracks, 0);
                                    }
                                }
                                if let Some(track) = self.queue.current().cloned() {
                                    let dur_ms = track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                    if let Ok(()) = self.load_and_play_meta(&track).await {
                                        track_started_at = Some(Instant::now());
                                        accumulated_position = Duration::ZERO;
                                        self.idle_since = None;
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
                                if let Some(ref sink) = self.sink {
                                    if let Ok(s) = sink.lock() {
                                        s.play();
                                    }
                                }
                                self.idle_since = None;
                                if track_started_at.is_none() {
                                    track_started_at = Some(Instant::now());
                                }
                                let mut state = self.state_tx.borrow().clone();
                                state.is_playing = true;
                                let _ = self.state_tx.send(state);
                            }
                        }
                        PlayerCommand::Pause => {
                            if let Some(ref sink) = self.sink {
                                if let Ok(s) = sink.lock() {
                                    s.pause();
                                }
                            }
                            if self.idle_since.is_none() {
                                self.idle_since = Some(Instant::now());
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
                            let is_empty = self.sink.as_ref().and_then(|s| s.lock().ok()).map(|s| s.empty()).unwrap_or(true);
                            let is_paused = self.sink.as_ref().and_then(|s| s.lock().ok()).map(|s| s.is_paused()).unwrap_or(false);
                            let mut is_playing = false;

                            if is_empty {
                                // Если ничего не играло, запускаем трек из очереди или подтягиваем с сервера
                                if self.queue.is_empty() {
                                    if let Ok(tracks) = self.loader.fetch_server_tracks().await {
                                        self.queue.set_queue(tracks, 0);
                                    }
                                }
                                if let Some(track) = self.queue.current().cloned() {
                                    let dur_ms = track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                    if let Ok(()) = self.load_and_play_meta(&track).await {
                                        track_started_at = Some(Instant::now());
                                        accumulated_position = Duration::ZERO;
                                        self.idle_since = None;
                                        let mut state = self.state_tx.borrow().clone();
                                        state.is_playing = true;
                                        state.position_ms = 0;
                                        state.duration_ms = dur_ms;
                                        state.current_track = Some(track);
                                        state.queue = self.queue.tracks();
                                        let _ = self.state_tx.send(state);
                                    }
                                }
                            } else if is_paused {
                                if let Some(ref sink) = self.sink {
                                    if let Ok(s) = sink.lock() {
                                        s.play();
                                        is_playing = true;
                                    }
                                }
                                self.idle_since = None;
                                if track_started_at.is_none() {
                                    track_started_at = Some(Instant::now());
                                }
                                let mut state = self.state_tx.borrow().clone();
                                state.is_playing = is_playing;
                                let _ = self.state_tx.send(state);
                            } else {
                                if let Some(ref sink) = self.sink {
                                    if let Ok(s) = sink.lock() {
                                        s.pause();
                                    }
                                }
                                if self.idle_since.is_none() {
                                    self.idle_since = Some(Instant::now());
                                }
                                if let Some(started) = track_started_at.take() {
                                    accumulated_position += started.elapsed();
                                }
                                let mut state = self.state_tx.borrow().clone();
                                state.is_playing = false;
                                let _ = self.state_tx.send(state);
                            }
                        }
                        PlayerCommand::Stop => {
                            self.release_audio();
                            track_started_at = None;
                            accumulated_position = Duration::ZERO;
                            self.current_audio_data = None;
                            self.current_track_id = None;
                            let mut state = self.state_tx.borrow().clone();
                            state.is_playing = false;
                            state.position_ms = 0;
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::Seek { position_ms } => {
                            info!("Получена команда перемотки (SEEK) на {} мс ({:.1} сек)", position_ms, position_ms as f64 / 1000.0);
                            let was_playing = self.sink.as_ref().and_then(|s| s.lock().ok()).map(|s| !s.is_paused() && !s.empty()).unwrap_or(false);

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
                            let vol = if self.is_ducked { self.config.ducking_volume } else { 1.0 };
                            if let Some(ref sink) = self.sink {
                                if let Ok(s) = sink.lock() {
                                    s.set_volume(vol);
                                }
                            }
                            set_hardware_volume(clamped);
                            save_persisted_volume(clamped);
                            let mut state = self.state_tx.borrow().clone();
                            state.volume = clamped;
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::Duck { ducked } => {
                            self.is_ducked = ducked;
                            let vol = if ducked { self.config.ducking_volume } else { 1.0 };
                            info!("🔉 Приглушение музыки для Алисы (Audio Ducking): ducked={}, volume={:.0}%", ducked, vol * 100.0);
                            if let Some(ref sink) = self.sink {
                                if let Ok(s) = sink.lock() {
                                    s.set_volume(vol);
                                }
                            }
                        }
                        PlayerCommand::StartFlow => {
                            info!("🌊 Активация режима Поток (Flow / Vibe Stream)");
                            self.is_flow_active = true;

                            let cur_id = self.state_tx.borrow().current_track.as_ref().map(|t| t.id);
                            if let Some(id) = cur_id {
                                if !self.recent_flow_ids.contains(&id) {
                                    self.recent_flow_ids.push(id);
                                }
                            }

                            match self.loader.fetch_next_stream_track(cur_id, &self.recent_flow_ids, "default").await {
                                Ok(next_track) => {
                                    info!("🎵 Получен трек для Потока: #{} - '{}' / '{}'", next_track.id, next_track.title, next_track.artist);
                                    self.recent_flow_ids.push(next_track.id);
                                    if self.recent_flow_ids.len() > 40 {
                                        self.recent_flow_ids.remove(0);
                                    }

                                    let dur_ms = next_track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                    let mut state = self.state_tx.borrow().clone();
                                    state.is_playing = true;
                                    state.is_flow = true;
                                    state.position_ms = 0;
                                    state.duration_ms = dur_ms;
                                    state.current_track = Some(next_track.clone());
                                    state.queue = vec![next_track.clone()];
                                    let _ = self.state_tx.send(state);

                                    if let Ok(()) = self.load_and_play_track(next_track.id).await {
                                        track_started_at = Some(Instant::now());
                                        accumulated_position = Duration::ZERO;
                                        let mut state = self.state_tx.borrow().clone();
                                        state.is_playing = true;
                                        let _ = self.state_tx.send(state);
                                    }
                                }
                                Err(e) => {
                                    error!("Не удалось получить следующий трек Потока: {:#}", e);
                                }
                            }
                        }
                        PlayerCommand::Next => {
                            info!("Команда переключения на следующий трек (Next)");
                            if self.is_flow_active {
                                let cur_id = self.state_tx.borrow().current_track.as_ref().map(|t| t.id);
                                if let Some(id) = cur_id {
                                    if !self.recent_flow_ids.contains(&id) {
                                        self.recent_flow_ids.push(id);
                                    }
                                }
                                match self.loader.fetch_next_stream_track(cur_id, &self.recent_flow_ids, "default").await {
                                    Ok(next_track) => {
                                        info!("🌊 [Поток] Переход к следующему треку: #{} - '{}'", next_track.id, next_track.title);
                                        self.recent_flow_ids.push(next_track.id);
                                        if self.recent_flow_ids.len() > 40 {
                                            self.recent_flow_ids.remove(0);
                                        }
                                        let dur_ms = next_track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                        let mut state = self.state_tx.borrow().clone();
                                        state.is_playing = true;
                                        state.is_flow = true;
                                        state.position_ms = 0;
                                        state.duration_ms = dur_ms;
                                        state.current_track = Some(next_track.clone());
                                        state.queue = vec![next_track.clone()];
                                        let _ = self.state_tx.send(state);

                                        if let Ok(()) = self.load_and_play_track(next_track.id).await {
                                            track_started_at = Some(Instant::now());
                                            accumulated_position = Duration::ZERO;
                                            let mut state = self.state_tx.borrow().clone();
                                            state.is_playing = true;
                                            let _ = self.state_tx.send(state);
                                        }
                                    }
                                    Err(e) => {
                                        error!("Ошибка получения следующего трека Потока: {:#}", e);
                                    }
                                }
                            } else {
                                if self.queue.is_empty() {
                                    if self.queue.is_shuffle() {
                                        if let Ok(tracks) = self.loader.fetch_shuffled_tracks(50, &[]).await {
                                            self.queue.set_queue(tracks, 0);
                                        }
                                    } else if let Ok(tracks) = self.loader.fetch_server_tracks().await {
                                        self.queue.set_queue(tracks, 0);
                                    }
                                } else if self.queue.is_shuffle() && self.queue.remaining() <= 2 {
                                    let ids = self.queue.track_ids();
                                    if let Ok(tracks) = self.loader.fetch_shuffled_tracks(30, &ids).await {
                                        self.queue.append_tracks(tracks);
                                    }
                                }
                                if let Some(next_track) = self.queue.next() {
                                    let dur_ms = next_track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);

                                    // Мгновенное оповещение сети и UI о выбранном треке (до скачивания аудио)
                                    let mut state = self.state_tx.borrow().clone();
                                    state.is_playing = true;
                                    state.position_ms = 0;
                                    state.duration_ms = dur_ms;
                                    state.current_track = Some(next_track.clone());
                                    state.queue = self.queue.tracks();
                                    let _ = self.state_tx.send(state);

                                    if let Ok(()) = self.load_and_play_meta(&next_track).await {
                                        track_started_at = Some(Instant::now());
                                        accumulated_position = Duration::ZERO;
                                        let mut state = self.state_tx.borrow().clone();
                                        state.is_playing = true;
                                        let _ = self.state_tx.send(state);
                                    }
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

                                // Мгновенное оповещение сети и UI о выбранном треке (до скачивания аудио)
                                let mut state = self.state_tx.borrow().clone();
                                state.is_playing = true;
                                state.position_ms = 0;
                                state.duration_ms = dur_ms;
                                state.current_track = Some(prev_track.clone());
                                state.queue = self.queue.tracks();
                                let _ = self.state_tx.send(state);

                                if let Ok(()) = self.load_and_play_meta(&prev_track).await {
                                    track_started_at = Some(Instant::now());
                                    accumulated_position = Duration::ZERO;
                                    let mut state = self.state_tx.borrow().clone();
                                    state.is_playing = true;
                                    let _ = self.state_tx.send(state);
                                }
                            }
                        }
                        PlayerCommand::ToggleShuffle => {
                            let new_shuffle = self.queue.toggle_shuffle();
                            if new_shuffle && self.queue.remaining() < 5 {
                                let cur_id = self.state_tx.borrow().current_track.as_ref().map(|t| t.id).unwrap_or(0);
                                if let Ok(tracks) = self.loader.fetch_shuffled_tracks(50, &[cur_id]).await {
                                    self.queue.append_tracks(tracks);
                                }
                            }
                            let mut state = self.state_tx.borrow().clone();
                            state.is_shuffle = new_shuffle;
                            state.queue = self.queue.tracks();
                            info!("🔀 Режим перемешивания (Shuffle): {}", new_shuffle);
                            let _ = self.state_tx.send(state);
                        }
                        PlayerCommand::SetShuffle { enabled } => {
                            self.queue.set_shuffle(enabled);
                            if enabled && self.queue.remaining() < 5 {
                                let cur_id = self.state_tx.borrow().current_track.as_ref().map(|t| t.id).unwrap_or(0);
                                if let Ok(tracks) = self.loader.fetch_shuffled_tracks(50, &[cur_id]).await {
                                    self.queue.append_tracks(tracks);
                                }
                            }
                            let mut state = self.state_tx.borrow().clone();
                            state.is_shuffle = enabled;
                            state.queue = self.queue.tracks();
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

                // Если плеер на паузе более 15 секунд или трек завершился — освобождаем ALSA устройство для сна (0% CPU)
                if let Some(idle_at) = self.idle_since {
                    if idle_at.elapsed() > Duration::from_secs(15) && !self.state_tx.borrow().is_playing {
                        self.release_audio();
                    }
                }

                let is_empty;
                let is_paused;
                if let Some(ref sink) = self.sink {
                    if let Ok(s) = sink.lock() {
                        is_empty = s.empty();
                        is_paused = s.is_paused();
                    } else {
                        continue;
                    }
                } else {
                    is_empty = true;
                    is_paused = false;
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
                                    if let Ok(()) = self.load_and_play_meta(&cur).await {
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

                            // Автоматический переход к следующему треку в бесконечном Потоке (Flow)
                            if self.is_flow_active {
                                let cur_id = state.current_track.as_ref().map(|t| t.id);
                                if let Some(id) = cur_id {
                                    if !self.recent_flow_ids.contains(&id) {
                                        self.recent_flow_ids.push(id);
                                    }
                                }
                                match self.loader.fetch_next_stream_track(cur_id, &self.recent_flow_ids, "default").await {
                                    Ok(next_track) => {
                                        info!("🌊 [Поток: автопереход] Запуск следующего трека: #{} - '{}'", next_track.id, next_track.title);
                                        self.recent_flow_ids.push(next_track.id);
                                        if self.recent_flow_ids.len() > 40 {
                                            self.recent_flow_ids.remove(0);
                                        }
                                        let dur_ms = next_track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                        state.is_playing = true;
                                        state.is_flow = true;
                                        state.position_ms = 0;
                                        state.duration_ms = dur_ms;
                                        state.current_track = Some(next_track.clone());
                                        state.queue = vec![next_track.clone()];
                                        let _ = self.state_tx.send(state.clone());

                                        if let Ok(()) = self.load_and_play_track(next_track.id).await {
                                            track_started_at = Some(Instant::now());
                                            accumulated_position = Duration::ZERO;
                                            continue;
                                        }
                                    }
                                    Err(e) => {
                                        error!("Ошибка автоперехода в Потоке: {:#}", e);
                                    }
                                }
                            }

                            // Если включен режим Shuffle и очередь заканчивается — подгружаем новые треки со всего сервера
                            if self.queue.is_shuffle() && self.queue.remaining() <= 2 {
                                let ids = self.queue.track_ids();
                                if let Ok(tracks) = self.loader.fetch_shuffled_tracks(30, &ids).await {
                                    self.queue.append_tracks(tracks);
                                }
                            }

                            // Трек завершился: пробуем автоматически перейти к следующему треку в очереди
                            if let Some(next_track) = self.queue.next() {
                                info!("Автовоспроизведение следующего трека: '{}'", next_track.title);
                                let dur_ms = next_track.duration.map(|d| (d * 1000.0) as u64).unwrap_or(0);
                                state.is_playing = true;
                                state.position_ms = 0;
                                state.duration_ms = dur_ms;
                                state.current_track = Some(next_track.clone());
                                state.queue = self.queue.tracks();
                                let _ = self.state_tx.send(state.clone());

                                if let Ok(()) = self.load_and_play_meta(&next_track).await {
                                    track_started_at = Some(Instant::now());
                                    accumulated_position = Duration::ZERO;
                                    continue;
                                }
                            }

                            state.is_playing = false;
                            track_started_at = None;
                            accumulated_position = Duration::ZERO;
                            if self.idle_since.is_none() {
                                self.idle_since = Some(Instant::now());
                            }
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

    /// Воспроизведение подготовленных байтов аудиопотока
    fn play_audio_bytes(&mut self, audio_bytes: bytes::Bytes, position_ms: u64, auto_play: bool) -> Result<()> {
        sync_hardware_routing(&mut self.jack_state, &self.config.audio_routing);
        self.current_audio_data = Some(audio_bytes.clone());

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

            let handle = self.ensure_audio_stream()?;
            if let Some(old_sink) = self.sink.take() {
                if let Ok(s) = old_sink.lock() {
                    s.stop();
                }
            }
            let new_sink = Sink::try_new(&handle)
                .context("Не удалось пересоздать Sink")?;
            let vol = if self.is_ducked { self.config.ducking_volume } else { 1.0 };
            new_sink.set_volume(vol);
            new_sink.append(buffered_source);
            if auto_play {
                new_sink.play();
                self.idle_since = None;
            } else {
                new_sink.pause();
                self.idle_since = Some(Instant::now());
            }
            self.sink = Some(Arc::new(Mutex::new(new_sink)));

            Ok(())
        }
    }

    /// Загрузка аудиоданных по сети и добавление в Sink плеера с указанной позиции и статусом auto_play
    async fn load_and_play_track_at(&mut self, track_id: i64, position_ms: u64, auto_play: bool) -> Result<()> {
        let audio_bytes = self.loader.download_stream(track_id).await?;
        self.current_track_id = Some(track_id);
        self.play_audio_bytes(audio_bytes, position_ms, auto_play)
    }

    /// Загрузка аудиопотока предпрослушивания кандидата и добавление в Sink плеера
    async fn load_and_play_candidate_preview(&mut self, candidate_id: &str, position_ms: u64, auto_play: bool) -> Result<()> {
        let audio_bytes = self.loader.download_candidate_preview(candidate_id).await?;
        self.current_track_id = None;
        self.play_audio_bytes(audio_bytes, position_ms, auto_play)
    }

    /// Загрузка аудиоданных по сети и немедленное воспроизведение с начала
    async fn load_and_play_track(&mut self, track_id: i64) -> Result<()> {
        self.load_and_play_track_at(track_id, 0, true).await
    }

    /// Загрузка и запуск трека на основе метаданных (с поддержкой предпрослушивания кандидатов)
    async fn load_and_play_meta(&mut self, track: &TrackMeta) -> Result<()> {
        if track.is_preview && track.candidate_id.is_some() {
            self.load_and_play_candidate_preview(track.candidate_id.as_deref().unwrap(), 0, true).await
        } else {
            self.load_and_play_track(track.id).await
        }
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
        let handle = self.ensure_audio_stream()?;
        if let Some(old_sink) = self.sink.take() {
            if let Ok(s) = old_sink.lock() {
                s.stop();
            }
        }
        let new_sink = Sink::try_new(&handle)
            .context("Не удалось пересоздать Sink для перемотки")?;
        let vol = if self.is_ducked { self.config.ducking_volume } else { 1.0 };
        new_sink.set_volume(vol);
        new_sink.append(buffered_source);
        if auto_play {
            new_sink.play();
            self.idle_since = None;
        } else {
            new_sink.pause();
            self.idle_since = Some(Instant::now());
        }
        self.sink = Some(Arc::new(Mutex::new(new_sink)));

        Ok(())
    }
}
