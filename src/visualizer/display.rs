use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, watch};
use tracing::info;

use crate::connect::protocol::PlayerState;
use crate::visualizer::led_clock::{DisplayFrame, LedHardwareDriver, TrackAnimStyle};
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
    Standby,
    Active,
    Flow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTrackStyle {
    Curtain,
    Tape,
    Slot,
    Chevron,
}

enum TrackAnimation {
    Next { start: Instant, style: ActiveTrackStyle },
    Prev { start: Instant, style: ActiveTrackStyle },
}

#[derive(Debug, Clone, Copy)]
enum TransitionSequence {
    Activation(Instant),
    Deactivation(Instant),
    Flow(Instant),
}

/// Менеджер экрана и кольца подсветки Яндекс Станции Мини 2:
/// - Эквалайзер частот (спектр FFT) во время воспроизведения на фронтальном экране
/// - Значок паузы || при постановке на паузу
/// - Индикация текущего уровня громкости (например " 70 ") на экране
/// - Аппаратная подсветка верхней кнопки (RGB-кольцо) цветом громкости (зеленый -> желтый -> оранжевый -> красный)
/// - Анимация перелистывания трека (шторка, бобины, барабан, шевроны, рандом)
/// - Цифровые часы HH:MM в режиме ожидания
pub struct DisplayManager {
    sample_buffer: Arc<Mutex<Vec<f32>>>,
    state_rx: watch::Receiver<PlayerState>,
    cmd_rx: mpsc::Receiver<DisplayCommand>,
    brightness: u8,
    timezone_offset_hours: i32,
    is_active_rx: watch::Receiver<bool>,
    track_style: TrackAnimStyle,
    random_counter: usize,
}

impl DisplayManager {
    pub fn new(
        sample_buffer: Arc<Mutex<Vec<f32>>>,
        state_rx: watch::Receiver<PlayerState>,
        cmd_rx: mpsc::Receiver<DisplayCommand>,
        brightness: u8,
        timezone_offset_hours: i32,
        is_active_rx: watch::Receiver<bool>,
        track_style: TrackAnimStyle,
    ) -> Self {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as usize;
        Self {
            sample_buffer,
            state_rx,
            cmd_rx,
            brightness,
            timezone_offset_hours,
            is_active_rx,
            track_style,
            random_counter: seed % 4,
        }
    }

    fn pick_track_style(&mut self) -> ActiveTrackStyle {
        match self.track_style {
            TrackAnimStyle::Curtain => ActiveTrackStyle::Curtain,
            TrackAnimStyle::Tape => ActiveTrackStyle::Tape,
            TrackAnimStyle::Slot => ActiveTrackStyle::Slot,
            TrackAnimStyle::Chevron => ActiveTrackStyle::Chevron,
            TrackAnimStyle::Random => {
                let choice = self.random_counter % 4;
                self.random_counter = self.random_counter.wrapping_add(1);
                match choice {
                    0 => ActiveTrackStyle::Curtain,
                    1 => ActiveTrackStyle::Tape,
                    2 => ActiveTrackStyle::Slot,
                    _ => ActiveTrackStyle::Chevron,
                }
            }
        }
    }

    /// Запуск цикла отрисовки экрана и кольца подсветки (~30 FPS)
    pub async fn run_loop(mut self) {
        info!("📺 Запуск менеджера экрана и подсветки кнопки Станции Мини 2 (30 FPS, стиль треков: {:?})", self.track_style);

        let mut clock_driver = LedHardwareDriver::new();
        let mut ring_driver = LedRingDriver::new();
        let mut analyzer = SpectrumAnalyzer::new(1024, 4);

        let mut ticker = tokio::time::interval(Duration::from_millis(33)); // ~30 кадров/сек

        let mut transition_seq: Option<TransitionSequence> = None;
        let mut volume_overlay: Option<(u32, Instant)> = None;
        let mut volume_ring: Option<((u8, u8, u8), Instant)> = None;
        let mut track_anim: Option<TrackAnimation> = None;
        let mut last_track_id: Option<i64> = None;
        let mut last_volume: f32 = self.state_rx.borrow().volume;
        let mut last_is_playing = self.state_rx.borrow().is_playing;
        let mut last_is_flow = self.state_rx.borrow().is_flow;
        let mut paused_at: Option<Instant> = None;
        let mut action_ring_flash: Option<((u8, u8, u8), Instant)> = None;
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
                        let style = self.pick_track_style();
                        track_anim = Some(TrackAnimation::Next { start: Instant::now(), style });
                    }
                    DisplayCommand::PrevTrack => {
                        clock_driver.invalidate();
                        let style = self.pick_track_style();
                        track_anim = Some(TrackAnimation::Prev { start: Instant::now(), style });
                    }
                    DisplayCommand::Pause => {
                        clock_driver.invalidate();
                        paused_at = Some(Instant::now());
                        action_ring_flash = Some(((0xE8, 0x00, 0xF3), Instant::now())); // Фиолетовая вспышка
                        equalizer_until = None;
                    }
                    DisplayCommand::Play => {
                        clock_driver.invalidate();
                        paused_at = None;
                        action_ring_flash = Some(((0x00, 0xEE, 0x80), Instant::now())); // Неоново-изумрудная вспышка
                        equalizer_until = Some(Instant::now() + Duration::from_secs(5));
                    }
                    DisplayCommand::Standby => {
                        clock_driver.invalidate();
                        transition_seq = Some(TransitionSequence::Deactivation(Instant::now()));
                    }
                    DisplayCommand::Active => {
                        clock_driver.invalidate();
                        transition_seq = Some(TransitionSequence::Activation(Instant::now()));
                    }
                    DisplayCommand::Flow => {
                        clock_driver.invalidate();
                        transition_seq = Some(TransitionSequence::Flow(Instant::now()));
                    }
                }
            }

            // 2. Отслеживание изменений состояния плеера
            let state = self.state_rx.borrow().clone();

            if state.is_flow && !last_is_flow {
                clock_driver.invalidate();
                transition_seq = Some(TransitionSequence::Flow(Instant::now()));
            }
            last_is_flow = state.is_flow;

            if state.is_playing != last_is_playing {
                clock_driver.invalidate();
                if state.is_playing {
                    equalizer_until = Some(Instant::now() + Duration::from_secs(5));
                    action_ring_flash = Some(((0x00, 0xEE, 0x80), Instant::now()));
                    paused_at = None;
                } else {
                    equalizer_until = None;
                    action_ring_flash = Some(((0xE8, 0x00, 0xF3), Instant::now()));
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
                    let style = self.pick_track_style();
                    track_anim = Some(TrackAnimation::Next { start: Instant::now(), style });
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
            let is_active = *self.is_active_rx.borrow();
            let mut skip_render = false;
            let mut frame = DisplayFrame::default();
            let mut is_animating_track = false;
            let mut is_in_transition = false;

            if let Some(trans) = transition_seq {
                match trans {
                    TransitionSequence::Activation(start) => {
                        let elapsed_ms = start.elapsed().as_millis();
                        if elapsed_ms < 1430 {
                            // Фаза 1: Бегущая строка "SENTINEL"
                            let step = ((elapsed_ms / 130) as usize).min(10);
                            frame = DisplayFrame::marquee("   SENTINEL   ", step);
                            is_in_transition = true;
                        } else if elapsed_ms < 3750 {
                            // Фаза 2: Живые глазки (открыты -> моргание -> влево -> вправо -> подмигивание -> счастливые ^^)
                            let eyes_ms = elapsed_ms - 1430;
                            frame = if eyes_ms < 520 {
                                DisplayFrame::open_eyes()
                            } else if eyes_ms < 670 {
                                DisplayFrame::blink_eyes()
                            } else if eyes_ms < 1070 {
                                DisplayFrame::look_left()
                            } else if eyes_ms < 1470 {
                                DisplayFrame::look_right()
                            } else if eyes_ms < 1820 {
                                DisplayFrame::wink_eyes()
                            } else {
                                DisplayFrame::happy_eyes()
                            };
                            is_in_transition = true;
                        } else {
                            transition_seq = None;
                            clock_driver.invalidate();

                            let sound_path = "/data/opt/sentinel-station/sounds/sentinel_online.wav";
                            if std::path::Path::new(sound_path).exists() {
                                tokio::spawn(async move {
                                    let _ = tokio::process::Command::new("aplay")
                                        .arg("-D")
                                        .arg("default")
                                        .arg(sound_path)
                                        .status()
                                        .await;
                                });
                            }
                        }
                    }
                    TransitionSequence::Deactivation(start) => {
                        let elapsed_ms = start.elapsed().as_millis();
                        if elapsed_ms < 1600 {
                            // Фаза 1: Засыпающие глазки (стадии 0..3: открыты -> опущены -> щелочки -> сон)
                            let stage = ((elapsed_ms / 400) as usize).min(3);
                            frame = DisplayFrame::sleepy_eyes(stage);
                            is_in_transition = true;
                        } else if elapsed_ms < 2720 {
                            // Фаза 2: Бегущая строка "ALICE"
                            let alice_ms = elapsed_ms - 1600;
                            let step = ((alice_ms / 140) as usize).min(7);
                            frame = DisplayFrame::marquee("   ALICE   ", step);
                            is_in_transition = true;
                        } else {
                            transition_seq = None;
                            ring_driver.turn_off();
                            clock_driver.invalidate();
                            skip_render = true;

                            // Сразу выводим текущие часы HH:MM, чтобы экран не оставался темным до смены минуты Алисы
                            let clock_frame = Self::render_clock_frame(self.timezone_offset_hours);
                            clock_driver.force_render(&clock_frame, 170);

                            let sound_path = "/data/opt/sentinel-station/sounds/alice_return.wav";
                            if std::path::Path::new(sound_path).exists() {
                                tokio::spawn(async move {
                                    let _ = tokio::process::Command::new("aplay")
                                        .arg("-D")
                                        .arg("default")
                                        .arg(sound_path)
                                        .status()
                                        .await;
                                });
                            }
                        }
                    }
                    TransitionSequence::Flow(start) => {
                        let elapsed_ms = start.elapsed().as_millis();
                        if elapsed_ms < 1800 {
                            frame = DisplayFrame::text("FLOW");
                            is_in_transition = true;
                        } else {
                            transition_seq = None;
                            ring_driver.turn_off();
                            clock_driver.invalidate();
                        }
                    }
                }
            }

            if !is_in_transition {
                if !is_active {
                    skip_render = true;
                } else {
                // Приоритет 1: Анимация переключения трека (Next/Prev) ~350 мс
                if let Some(anim) = &track_anim {
                    match anim {
                        TrackAnimation::Next { start, style } => {
                            let elapsed_ms = start.elapsed().as_millis();
                            let (step, max_steps) = match style {
                                ActiveTrackStyle::Curtain => ((elapsed_ms / 45) as usize, 8),
                                ActiveTrackStyle::Tape => ((elapsed_ms / 45) as usize, 8),
                                ActiveTrackStyle::Slot => ((elapsed_ms / 45) as usize, 8),
                                ActiveTrackStyle::Chevron => ((elapsed_ms / 60) as usize, 6),
                            };
                            if step < max_steps {
                                frame = match style {
                                    ActiveTrackStyle::Curtain => DisplayFrame::curtain_track(step, true),
                                    ActiveTrackStyle::Tape => DisplayFrame::tape_track(step, true),
                                    ActiveTrackStyle::Slot => DisplayFrame::slot_track(step, true),
                                    ActiveTrackStyle::Chevron => DisplayFrame::chevron_track(step, true),
                                };
                                is_animating_track = true;
                            }
                        }
                        TrackAnimation::Prev { start, style } => {
                            let elapsed_ms = start.elapsed().as_millis();
                            let (step, max_steps) = match style {
                                ActiveTrackStyle::Curtain => ((elapsed_ms / 45) as usize, 8),
                                ActiveTrackStyle::Tape => ((elapsed_ms / 45) as usize, 8),
                                ActiveTrackStyle::Slot => ((elapsed_ms / 45) as usize, 8),
                                ActiveTrackStyle::Chevron => ((elapsed_ms / 60) as usize, 6),
                            };
                            if step < max_steps {
                                frame = match style {
                                    ActiveTrackStyle::Curtain => DisplayFrame::curtain_track(step, false),
                                    ActiveTrackStyle::Tape => DisplayFrame::tape_track(step, false),
                                    ActiveTrackStyle::Slot => DisplayFrame::slot_track(step, false),
                                    ActiveTrackStyle::Chevron => DisplayFrame::chevron_track(step, false),
                                };
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
            }
        }

            // 4. Отрисовка кадра на фронтальном LED-экране Станции
            if !skip_render {
                if is_in_transition {
                    clock_driver.force_render(&frame, self.brightness);
                } else {
                    clock_driver.render(&frame, self.brightness);
                }
            }

            // 5. Отрисовка подсветки кнопки (RGB-кольцо)
            let now = Instant::now();
            if is_in_transition {
                if let Some(trans) = transition_seq {
                    match trans {
                        TransitionSequence::Activation(start) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            if elapsed_ms < 1430 {
                                // Кометный вращающийся маяк Sentinel (циан + пурпур)
                                let phase = ((elapsed_ms / 60) % 6) as usize;
                                let mut ring_colors = [(0u8, 0u8, 0u8); 6];
                                ring_colors[phase] = (0x00, 0xC8, 0xFF);
                                ring_colors[(phase + 1) % 6] = (0xDC, 0x01, 0xEF);
                                let trail = (phase + 5) % 6;
                                ring_colors[trail] = (0x00, 0x30, 0x50);
                                ring_driver.set_colors(&ring_colors);
                            } else if elapsed_ms < 3750 {
                                let eyes_ms = elapsed_ms - 1430;
                                if eyes_ms < 520 {
                                    ring_driver.set_solid(0, 150, 240);
                                } else if eyes_ms < 670 {
                                    ring_driver.set_solid(0, 30, 60);
                                } else if eyes_ms < 1070 {
                                    // Взгляд влево: левая сторона кольца
                                    ring_driver.set_colors(&[
                                        (0, 180, 220),
                                        (0, 0, 0),
                                        (0, 0, 0),
                                        (0, 0, 0),
                                        (0, 180, 220),
                                        (0, 180, 220),
                                    ]);
                                } else if eyes_ms < 1470 {
                                    // Взгляд вправо: правая сторона кольца
                                    ring_driver.set_colors(&[
                                        (0, 0, 0),
                                        (0, 180, 220),
                                        (0, 180, 220),
                                        (0, 180, 220),
                                        (0, 0, 0),
                                        (0, 0, 0),
                                    ]);
                                } else if eyes_ms < 1820 {
                                    // Подмигивание: золотая теплая вспышка
                                    ring_driver.set_solid(255, 190, 40);
                                } else {
                                    // Счастливые глазки ^^: изумрудно-бирюзовый цвет
                                    ring_driver.set_solid(0, 255, 120);
                                }
                            }
                        }
                        TransitionSequence::Deactivation(start) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            if elapsed_ms < 1600 {
                                let stage = ((elapsed_ms / 400) as usize).min(3);
                                match stage {
                                    0 => ring_driver.set_solid(255, 120, 0),
                                    1 => ring_driver.set_solid(160, 60, 0),
                                    2 => ring_driver.set_solid(70, 20, 0),
                                    _ => ring_driver.set_solid(20, 5, 0),
                                }
                            } else if elapsed_ms < 2720 {
                                let alice_ms = elapsed_ms - 1600;
                                let phase = ((alice_ms / 60) % 6) as usize;
                                let mut ring_colors = [(0u8, 0u8, 0u8); 6];
                                ring_colors[phase] = (0x9E, 0x00, 0xFF);
                                ring_colors[(phase + 1) % 6] = (0x50, 0x00, 0x90);
                                ring_driver.set_colors(&ring_colors);
                            }
                        }
                        TransitionSequence::Flow(start) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            if elapsed_ms < 1800 {
                                // Волновой поток: циан-изумрудная волна по кольцу
                                let phase = ((elapsed_ms / 80) % 6) as usize;
                                let mut ring_colors = [(0u8, 0u8, 0u8); 6];
                                ring_colors[phase] = (0x00, 0xF5, 0xD4); // Бирюзовый
                                ring_colors[(phase + 1) % 6] = (0x00, 0xEE, 0x80); // Изумрудный
                                ring_colors[(phase + 5) % 6] = (0x00, 0x50, 0x40); // Шлейф
                                ring_driver.set_colors(&ring_colors);
                            }
                        }
                    }
                }
            } else if skip_render && volume_ring.is_none() {
                // В спящем режиме подсветка выключена
                ring_driver.turn_off();
            } else if is_animating_track {
                // Анимация перелистывания трека на кольце (согласованная со стилем экрана)
                if let Some(anim) = &track_anim {
                    let (elapsed_ms, is_next, style) = match anim {
                        TrackAnimation::Next { start, style } => (start.elapsed().as_millis(), true, *style),
                        TrackAnimation::Prev { start, style } => (start.elapsed().as_millis(), false, *style),
                    };
                    match style {
                        ActiveTrackStyle::Curtain => {
                            let head = if is_next { ((elapsed_ms / 50) % 6) as usize } else { ((6 - ((elapsed_ms / 50) % 6)) % 6) as usize };
                            let mut ring_colors = [(0u8, 0u8, 0u8); 6];
                            let color = if is_next { (0xDC, 0x01, 0xEF) } else { (0x00, 0xC8, 0xFF) };
                            ring_colors[head] = color;
                            let trail = if is_next { (head + 5) % 6 } else { (head + 1) % 6 };
                            ring_colors[trail] = (color.0 / 4, color.1 / 4, color.2 / 4);
                            ring_driver.set_colors(&ring_colors);
                        }
                        ActiveTrackStyle::Tape => {
                            let head = if is_next { ((elapsed_ms / 35) % 6) as usize } else { ((6 - ((elapsed_ms / 35) % 6)) % 6) as usize };
                            let mut ring_colors = [(0u8, 0u8, 0u8); 6];
                            ring_colors[head] = (255, 180, 0);
                            ring_colors[(head + 3) % 6] = (180, 0, 255);
                            ring_driver.set_colors(&ring_colors);
                        }
                        ActiveTrackStyle::Slot => {
                            let phase = ((elapsed_ms / 60) % 6) as usize;
                            let mut ring_colors = [(0u8, 0u8, 0u8); 6];
                            for i in 0..=phase.min(5) {
                                ring_colors[i] = if is_next { (0, 220, 180) } else { (255, 100, 0) };
                            }
                            ring_driver.set_colors(&ring_colors);
                        }
                        ActiveTrackStyle::Chevron => {
                            let head = if is_next { ((elapsed_ms / 60) % 6) as usize } else { ((6 - ((elapsed_ms / 60) % 6)) % 6) as usize };
                            let mut ring_colors = [(0u8, 0u8, 0u8); 6];
                            ring_colors[head] = (0xDC, 0x01, 0xEF);
                            let trail = if is_next { (head + 5) % 6 } else { (head + 1) % 6 };
                            ring_colors[trail] = (0x30, 0x00, 0x40);
                            ring_driver.set_colors(&ring_colors);
                        }
                    }
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
            } else if let Some(((r, g, b), start)) = action_ring_flash {
                // Мягкая вспышка кольца (Play: неоново-изумрудный, Pause: фиолетовый)
                let elapsed = start.elapsed().as_millis();
                if elapsed < 450 {
                    let fade = 1.0 - (elapsed as f32 / 450.0);
                    let fr = (r as f32 * fade) as u8;
                    let fg = (g as f32 * fade) as u8;
                    let fb = (b as f32 * fade) as u8;
                    ring_driver.set_solid(fr, fg, fb);
                } else {
                    action_ring_flash = None;
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
