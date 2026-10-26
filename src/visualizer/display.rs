use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};
use tracing::info;

use crate::connect::protocol::PlayerState;
use crate::visualizer::led_clock::{DisplayFrame, LedHardwareDriver};
use crate::visualizer::led_ring::{volume_to_color, LedRingDriver};
use crate::visualizer::SpectrumAnalyzer;

/// Команды управления отображением на экране и подсветке Станции
#[derive(Debug, Clone, Copy)]
pub enum DisplayCommand {
    NextTrack,
    PrevTrack,
    Volume(f32),
    Pause,
    Play,
}

enum TrackAnimation {
    Next(Instant),
    Prev(Instant),
}

/// Менеджер экрана и кольца подсветки Яндекс Станции Мини 2:
/// - Эквалайзер частот (спектр FFT) во время воспроизведения на фронтальном экране
/// - Значок паузы || при постановке на паузу
/// - Индикация текущего уровня громкости (например " 70 ") на экране
/// - Аппаратная подсветка верхней кнопки (RGB-кольцо) цветом громкости (зеленый -> желтый -> оранжевый -> красный)
/// - Анимация перелистывания трека (>> или << на часах и круговое фиолетовое вращение на кнопке)
/// - Цифровые часы HH:MM в режиме ожидания
pub struct DisplayManager {
    sample_buffer: Arc<Mutex<Vec<f32>>>,
    state_rx: watch::Receiver<PlayerState>,
    cmd_rx: mpsc::Receiver<DisplayCommand>,
    brightness: u8,
    timezone_offset_hours: i32,
}

impl DisplayManager {
    pub fn new(
        sample_buffer: Arc<Mutex<Vec<f32>>>,
        state_rx: watch::Receiver<PlayerState>,
        cmd_rx: mpsc::Receiver<DisplayCommand>,
        brightness: u8,
        timezone_offset_hours: i32,
    ) -> Self {
        Self {
            sample_buffer,
            state_rx,
            cmd_rx,
            brightness,
            timezone_offset_hours,
        }
    }

    /// Запуск цикла отрисовки экрана и кольца подсветки (~30 FPS)
    pub async fn run_loop(mut self) {
        info!("📺 Запуск менеджера экрана и подсветки кнопки Станции Мини 2 (30 FPS)");

        let mut clock_driver = LedHardwareDriver::new();
        let mut ring_driver = LedRingDriver::new();
        let mut analyzer = SpectrumAnalyzer::new(1024, 4);

        let mut ticker = tokio::time::interval(Duration::from_millis(33)); // ~30 кадров/сек

        let mut volume_overlay: Option<(u32, Instant)> = None;
        let mut volume_ring: Option<((u8, u8, u8), Instant)> = None;
        let mut track_anim: Option<TrackAnimation> = None;
        let mut last_track_id: Option<i64> = None;
        let mut last_volume: f32 = self.state_rx.borrow().volume;
        let mut last_is_playing = self.state_rx.borrow().is_playing;
        let mut paused_at: Option<Instant> = None;
        let mut equalizer_until: Option<Instant> = if last_is_playing {
            Some(Instant::now() + Duration::from_secs(5))
        } else {
            None
        };
        let mut showing_equalizer = false;

        loop {
            ticker.tick().await;

            // 1. Обработка прямых команд индикации
            while let Ok(cmd) = self.cmd_rx.try_recv() {
                match cmd {
                    DisplayCommand::Volume(vol) => {
                        clock_driver.invalidate();
                        let pct = (vol.clamp(0.0, 1.0) * 100.0).round() as u32;
                        let until = Instant::now() + Duration::from_millis(1800);
                        volume_overlay = Some((pct, until));
                        volume_ring = Some((volume_to_color(vol), until));
                        last_volume = vol;
                    }
                    DisplayCommand::NextTrack => {
                        clock_driver.invalidate();
                        track_anim = Some(TrackAnimation::Next(Instant::now()));
                    }
                    DisplayCommand::PrevTrack => {
                        clock_driver.invalidate();
                        track_anim = Some(TrackAnimation::Prev(Instant::now()));
                    }
                    DisplayCommand::Pause => {
                        clock_driver.invalidate();
                        paused_at = Some(Instant::now());
                        equalizer_until = None;
                    }
                    DisplayCommand::Play => {
                        clock_driver.invalidate();
                        paused_at = None;
                        equalizer_until = Some(Instant::now() + Duration::from_secs(5));
                    }
                }
            }

            // 2. Отслеживание изменений состояния плеера
            let state = self.state_rx.borrow().clone();

            if state.is_playing != last_is_playing {
                clock_driver.invalidate();
                if state.is_playing {
                    equalizer_until = Some(Instant::now() + Duration::from_secs(5));
                    paused_at = None;
                } else {
                    equalizer_until = None;
                    paused_at = Some(Instant::now());
                }
                last_is_playing = state.is_playing;
            }

            // Детекция изменения громкости (например, через слайдер в веб-интерфейсе или телефоне)
            if (state.volume - last_volume).abs() > 0.02 && volume_overlay.is_none() {
                clock_driver.invalidate();
                let pct = (state.volume.clamp(0.0, 1.0) * 100.0).round() as u32;
                let until = Instant::now() + Duration::from_millis(1800);
                volume_overlay = Some((pct, until));
                volume_ring = Some((volume_to_color(state.volume), until));
                last_volume = state.volume;
            }

            // Детекция автоматической смены трека в очереди (показываем эквалайзер на 5 секунд)
            let current_track_id = state.current_track.as_ref().map(|t| t.id);
            if current_track_id != last_track_id {
                if last_track_id.is_some() && track_anim.is_none() {
                    clock_driver.invalidate();
                    track_anim = Some(TrackAnimation::Next(Instant::now()));
                }
                if state.is_playing {
                    equalizer_until = Some(Instant::now() + Duration::from_secs(5));
                }
                last_track_id = current_track_id;
            }

            // Обновление таймера паузы
            if !state.is_playing && paused_at.is_none() {
                clock_driver.invalidate();
                paused_at = Some(Instant::now());
            } else if state.is_playing {
                paused_at = None;
            }

            // 3. Выбор кадра для отрисовки часов в зависимости от приоритета
            let mut frame = DisplayFrame::default();

            // Приоритет 1: Анимация переключения трека (Next/Prev) ~350 мс
            let mut is_animating_track = false;
            if let Some(anim) = &track_anim {
                match anim {
                    TrackAnimation::Next(start) => {
                        let elapsed_ms = start.elapsed().as_millis();
                        let step = (elapsed_ms / 70) as usize;
                        if step < 5 {
                            frame = DisplayFrame::next_track(step);
                            is_animating_track = true;
                        }
                    }
                    TrackAnimation::Prev(start) => {
                        let elapsed_ms = start.elapsed().as_millis();
                        let step = (elapsed_ms / 70) as usize;
                        if step < 5 {
                            frame = DisplayFrame::prev_track(step);
                            is_animating_track = true;
                        }
                    }
                }
            }
            if !is_animating_track && track_anim.is_some() {
                clock_driver.invalidate();
                track_anim = None;
            }

            // Приоритет 2: Оверлей громкости (1.8 сек после нажатия + / -)
            let mut is_showing_volume = false;
            if !is_animating_track {
                if let Some((vol_pct, until)) = volume_overlay {
                    if Instant::now() < until {
                        frame = DisplayFrame::volume(vol_pct);
                        is_showing_volume = true;
                    } else {
                        clock_driver.invalidate();
                        volume_overlay = None;
                    }
                }
            }

            // Приоритет 3: Воспроизведение трека -> Эквалайзер частот (первые 5 секунд) или Часы
            if !is_animating_track && !is_showing_volume {
                let should_show_equalizer = state.is_playing
                    && equalizer_until.map(|u| Instant::now() < u).unwrap_or(false);

                // При переходе между эквалайзером и часами сбрасываем кэш сегментов для чистой перерисовки
                if should_show_equalizer != showing_equalizer {
                    clock_driver.invalidate();
                    showing_equalizer = should_show_equalizer;
                }

                if should_show_equalizer {
                    let samples = if let Ok(buf) = self.sample_buffer.lock() {
                        buf.clone()
                    } else {
                        Vec::new()
                    };

                    let bands = analyzer.compute(&samples);
                    frame = DisplayFrame::spectrum(&bands);
                } else if !state.is_playing {
                    // Пауза или режим ожидания
                    let pause_duration = paused_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO);
                    if pause_duration < Duration::from_millis(2500) {
                        // Первые 2.5 секунды после паузы — четкий значок паузы ||
                        frame = DisplayFrame::pause();
                    } else {
                        // В режиме ожидания — часы HH:MM
                        frame = Self::render_clock_frame(self.timezone_offset_hours);
                    }
                } else {
                    // Музыка играет, но 5 секунд эквалайзера уже прошли -> показываем часы HH:MM!
                    frame = Self::render_clock_frame(self.timezone_offset_hours);
                }
            }

            // 4. Отрисовка кадра на фронтальном LED-экране Станции
            clock_driver.render(&frame, self.brightness);

            // 5. Отрисовка подсветки кнопки (RGB-кольцо)
            let now = Instant::now();
            if is_animating_track {
                // Анимация перелистывания трека на кольце: вращающийся фиолетовый огонек
                if let Some(anim) = &track_anim {
                    let (elapsed_ms, is_next) = match anim {
                        TrackAnimation::Next(start) => (start.elapsed().as_millis(), true),
                        TrackAnimation::Prev(start) => (start.elapsed().as_millis(), false),
                    };
                    let head = if is_next {
                        ((elapsed_ms / 60) % 6) as usize
                    } else {
                        ((6 - ((elapsed_ms / 60) % 6)) % 6) as usize
                    };
                    let mut ring_colors = [(0u8, 0u8, 0u8); 6];
                    ring_colors[head] = (0xDC, 0x01, 0xEF);
                    let trail = if is_next { (head + 5) % 6 } else { (head + 1) % 6 };
                    ring_colors[trail] = (0x30, 0x00, 0x40);
                    ring_driver.set_colors(&ring_colors);
                }
            } else if let Some(((r, g, b), until)) = volume_ring {
                // Подсветка громкости: удержание цвета и плавное затухание
                if now < until {
                    ring_driver.set_solid(r, g, b);
                } else if now < until + Duration::from_millis(400) {
                    let fade = 1.0 - (now - until).as_secs_f32() / 0.4;
                    let fr = (r as f32 * fade) as u8;
                    let fg = (g as f32 * fade) as u8;
                    let fb = (b as f32 * fade) as u8;
                    ring_driver.set_solid(fr, fg, fb);
                } else {
                    volume_ring = None;
                    ring_driver.turn_off();
                }
            } else if let Some(p_time) = paused_at {
                // Короткая мягкая вспышка фиолетового цвета при нажатии на паузу
                let elapsed = p_time.elapsed().as_millis();
                if elapsed < 400 {
                    let fade = 1.0 - (elapsed as f32 / 400.0);
                    let pr = (0xE8 as f32 * fade) as u8;
                    let pb = (0xF3 as f32 * fade) as u8;
                    ring_driver.set_solid(pr, 0, pb);
                } else {
                    ring_driver.turn_off();
                }
            } else {
                ring_driver.turn_off();
            }
        }
    }

    /// Формирование кадра цифровых часов с мигающим секундным двоеточием
    fn render_clock_frame(timezone_offset_hours: i32) -> DisplayFrame {
        let total_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64
            + (timezone_offset_hours as i64 * 3600);
        let total_mins = total_secs / 60;
        let hours = ((total_mins / 60) % 24) as u32;
        let minutes = (total_mins % 60) as u32;
        let colon = (total_secs % 2) == 0;
        DisplayFrame::clock(hours, minutes, colon)
    }
}
