use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::connect::protocol::{PlayerCommand, PlayerState};
use crate::visualizer::DisplayCommand;

#[cfg(unix)]
fn get_file_ino(file: &File) -> Option<u64> {
    use std::os::unix::io::AsRawFd;
    let mut stat_buf = std::mem::MaybeUninit::<libc::stat>::uninit();
    let res = unsafe { libc::fstat(file.as_raw_fd(), stat_buf.as_mut_ptr()) };
    if res == 0 {
        let s = unsafe { stat_buf.assume_init() };
        Some(s.st_ino as u64)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn get_file_ino(_file: &File) -> Option<u64> {
    None
}

#[cfg(unix)]
fn get_path_stat(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat_buf = std::mem::MaybeUninit::<libc::stat>::uninit();
    let res = unsafe { libc::stat(c_path.as_ptr(), stat_buf.as_mut_ptr()) };
    if res == 0 {
        let s = unsafe { stat_buf.assume_init() };
        Some((s.st_ino as u64, s.st_size as u64))
    } else {
        None
    }
}

#[cfg(not(unix))]
fn get_path_stat(_path: &std::path::Path) -> Option<(u64, u64)> {
    None
}

/// Локальный перехватчик голосовых команд Алисы через журнал событий Quasar (maind)
pub struct VoiceInterceptor {
    player_tx: mpsc::Sender<PlayerCommand>,
    state_rx: watch::Receiver<PlayerState>,
    display_tx: mpsc::Sender<DisplayCommand>,
    is_active_tx: watch::Sender<bool>,
    is_ducked: Arc<AtomicBool>,
    log_path: PathBuf,
}

impl VoiceInterceptor {
    pub fn new(
        player_tx: mpsc::Sender<PlayerCommand>,
        state_rx: watch::Receiver<PlayerState>,
        display_tx: mpsc::Sender<DisplayCommand>,
        is_active_tx: watch::Sender<bool>,
    ) -> Self {
        Self::ensure_mediad_running();
        Self {
            player_tx,
            state_rx,
            display_tx,
            is_active_tx,
            is_ducked: Arc::new(AtomicBool::new(false)),
            log_path: PathBuf::from("/data/quasar/daemons_logs/maind.log"),
        }
    }

    /// Поиск PID медиадемона Яндекс.Музыки (maind --mediad) через /proc
    pub fn find_mediad_pid() -> Option<u32> {
        let entries = fs::read_dir("/proc").ok()?;
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                if let Ok(pid) = name.parse::<u32>() {
                    let cmdline_path = entry.path().join("cmdline");
                    if let Ok(cmdline) = fs::read_to_string(cmdline_path) {
                        if cmdline.contains("--mediad") {
                            return Some(pid);
                        }
                    }
                }
            }
        }
        None
    }

    /// Проверка и разморозка mediad через SIGCONT (на случай, если он был заморожен предыдущей версией)
    pub fn ensure_mediad_running() {
        if let Some(pid) = Self::find_mediad_pid() {
            unsafe {
                libc::kill(pid as i32, libc::SIGCONT);
            }
        }
    }

    /// Безопасная остановка воспроизведения и отмена диалога Алисы через Quasar CLI
    /// (Не замораживает mediad через SIGSTOP, предотвращая зависание экрана и IPC)
    pub fn stop_alice_playback() {
        tokio::spawn(async {
            let res = tokio::process::Command::new("sh")
                .arg("-c")
                .arg("{ sleep 0.2; echo o; sleep 0.1; echo c; sleep 0.1; echo x; } | LD_LIBRARY_PATH=/system/vendor/quasar/libs /system/vendor/quasar/maind --cli >/dev/null 2>&1")
                .status()
                .await;
            if let Ok(status) = res {
                debug!("Штатное воспроизведение и диалог Алисы сброшены (code: {:?})", status.code());
            }
        });
    }

    /// Запуск фонового мониторинга журнала голосового ввода
    pub fn start(self) {
        tokio::spawn(async move {
            self.run_loop().await;
        });
    }

    async fn run_loop(self) {
        info!("🎤 Запуск перехватчика голосовых команд Алисы (мониторинг: {})", self.log_path.display());

        // Ожидаем появления файла лога, если система только загружается
        while !self.log_path.exists() {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        let mut file = match File::open(&self.log_path) {
            Ok(f) => f,
            Err(e) => {
                warn!("Не удалось открыть лог Quasar {}: {}. Голосовой перехват отключен.", self.log_path.display(), e);
                return;
            }
        };

        let mut in_call = false;
        let mut in_alarm_or_timer = false;
        let mut alarm_start_time: Option<Instant> = None;

        // Проверяем последние 8 КБ лога, чтобы узнать, не идет ли уже звонок или будильник в момент запуска
        let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
        let check_offset = file_len.saturating_sub(8192);
        let _ = file.seek(SeekFrom::Start(check_offset));
        let mut init_reader = BufReader::new(&file);
        let mut init_line = String::new();
        while init_reader.read_line(&mut init_line).unwrap_or(0) > 0 {
            let lower = init_line.to_lowercase();
            if Self::is_call_start(&lower) {
                in_call = true;
            } else if Self::is_call_end(&lower) {
                in_call = false;
            }
            if Self::is_alarm_or_timer_start(&lower) {
                in_alarm_or_timer = true;
                alarm_start_time = Some(Instant::now());
            } else if Self::is_alarm_or_timer_end(&lower) {
                in_alarm_or_timer = false;
                alarm_start_time = None;
            }
            init_line.clear();
        }
        if in_call || in_alarm_or_timer {
            info!("📞 При запуске обнаружен активный звонок или будильник/таймер — освобождаем кнопки для Алисы");
            let _ = self.is_active_tx.send(false);
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::PassThrough).await;
            });
        }

        // Перемещаемся в конец файла, чтобы обрабатывать только новые голосовые события
        let _ = file.seek(SeekFrom::End(0));
        let mut curr_file_ino = get_file_ino(&file);
        let mut reader = BufReader::new(file);

        let mut last_phrase = String::new();
        let mut last_phrase_time = Instant::now();
        let mut last_rotation_check = Instant::now();
        let mut duck_start_time: Option<Instant> = None;

        let mut line_buffer = String::new();

        loop {
            line_buffer.clear();
            match reader.read_line(&mut line_buffer) {
                Ok(0) => {
                    // Конец файла (EOF): проверяем ротацию файла не чаще одного раза в 5 секунд (защита от statx flood)
                    let now = Instant::now();
                    if now.duration_since(last_rotation_check) >= Duration::from_secs(5) {
                        last_rotation_check = now;

                        let should_reopen = match get_path_stat(&self.log_path) {
                            Some((path_ino, path_size)) => {
                                let ino_changed = match (Some(path_ino), curr_file_ino) {
                                    (Some(p), Some(c)) => p != c,
                                    _ => false,
                                };
                                let size_dropped = match reader.stream_position() {
                                    Ok(pos) => path_size < pos,
                                    Err(_) => false,
                                };
                                ino_changed || size_dropped
                            }
                            None => false,
                        };

                        if should_reopen {
                            if let Ok(mut new_file) = File::open(&self.log_path) {
                                curr_file_ino = get_file_ino(&new_file);
                                if let Ok(pos) = new_file.seek(SeekFrom::End(0)) {
                                    if pos > 16384 {
                                        let _ = new_file.seek(SeekFrom::End(0));
                                    }
                                }
                                reader = BufReader::new(new_file);
                            }
                        }
                    }
                    // Защита от зависания приглушения: авто-восстановление громкости через 15 сек
                    if self.is_ducked.load(Ordering::SeqCst) {
                        if let Some(start_t) = duck_start_time {
                            if start_t.elapsed() > Duration::from_secs(15) {
                                self.is_ducked.store(false, Ordering::SeqCst);
                                duck_start_time = None;
                                info!("⏰ Таймаут активности Алисы (15 сек) истек: восстанавливаем громкость и экран Sentinel Music");
                                let player_tx = self.player_tx.clone();
                                let display_tx = self.display_tx.clone();
                                tokio::spawn(async move {
                                    let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                                    let _ = display_tx.send(DisplayCommand::PassThroughResume).await;
                                });
                            }
                        }
                    }

                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                Ok(_) => {
                    let line = line_buffer.trim();
                    if line.is_empty() {
                        continue;
                    }

                    let lower = line.to_lowercase();
                    let prev_busy = in_call || in_alarm_or_timer;

                    // 0. Отслеживание активности Алисы и приглушение звука (Audio Ducking)
                    let is_alice_activation = (line.contains("onPhraseSpotted:") && line.contains("activation"))
                        || line.contains("onAliceStateChanged: LISTENING")
                        || line.contains("onAliceStateChanged: BUSY")
                        || line.contains("onAliceStateChanged: SPEAKING");

                    let is_alice_idle = line.contains("onAliceStateChanged: IDLE")
                        || line.contains("stopConversation");

                    if is_alice_activation {
                        if !self.is_ducked.swap(true, Ordering::SeqCst) {
                            duck_start_time = Some(Instant::now());
                            info!("🎙️ Обнаружена активность Алисы: приглушаем Sentinel Music и уступаем экран Алисе");
                            let player_tx = self.player_tx.clone();
                            let display_tx = self.display_tx.clone();
                            tokio::spawn(async move {
                                let _ = player_tx.send(PlayerCommand::Duck { ducked: true }).await;
                                let _ = display_tx.send(DisplayCommand::PassThrough).await;
                            });
                        }
                    } else if is_alice_idle {
                        if self.is_ducked.swap(false, Ordering::SeqCst) {
                            duck_start_time = None;
                            info!("🔊 Диалог Алисы завершен: восстанавливаем громкость Sentinel Music и возвращаем экран");
                            let player_tx = self.player_tx.clone();
                            let display_tx = self.display_tx.clone();
                            tokio::spawn(async move {
                                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                                let _ = display_tx.send(DisplayCommand::PassThroughResume).await;
                            });
                        }
                    }

                    // 1. Отслеживание состояния телефонных звонков (PJSIP / WebRTC)
                    if Self::is_call_start(&lower) {
                        if !in_call {
                            in_call = true;
                            info!("📞 Обнаружен входящий или исходящий звонок на Станции! (лог: {})", line);
                        }
                    } else if Self::is_call_end(&lower) {
                        if in_call {
                            in_call = false;
                            info!("📴 Телефонный вызов на Станции завершен (лог: {})", line);
                        }
                    }

                    // 2. Отслеживание срабатывания будильников и таймеров
                    if Self::is_alarm_or_timer_start(&lower) {
                        if !in_alarm_or_timer {
                            in_alarm_or_timer = true;
                            alarm_start_time = Some(Instant::now());
                            info!("⏰ Сработал будильник или таймер на Станции! (лог: {})", line);
                        }
                    } else if Self::is_alarm_or_timer_end(&lower) {
                        if in_alarm_or_timer {
                            in_alarm_or_timer = false;
                            alarm_start_time = None;
                            info!("🔔 Будильник или таймер на Станции остановлен (лог: {})", line);
                        }
                    }

                    // Авто-сброс будильника/таймера по таймауту (180 сек), если событие остановки не попало в лог
                    if in_alarm_or_timer {
                        if let Some(start_t) = alarm_start_time {
                            if start_t.elapsed() > Duration::from_secs(180) {
                                in_alarm_or_timer = false;
                                alarm_start_time = None;
                                info!("⏰ Таймаут активности будильника (3 мин) истек — сбрасываем флаг занятости");
                            }
                        }
                    }

                    let curr_busy = in_call || in_alarm_or_timer;

                    if !prev_busy && curr_busy {
                        info!("🔓 Освобождаем кнопки для Алисы (звонок/будильник/таймер) и приостанавливаем Sentinel Music");
                        let _ = self.is_active_tx.send(false);
                        let player_tx = self.player_tx.clone();
                        let display_tx = self.display_tx.clone();
                        tokio::spawn(async move {
                            let _ = display_tx.send(DisplayCommand::PassThrough).await;
                            let _ = player_tx.send(PlayerCommand::Pause).await;
                        });
                    } else if prev_busy && !curr_busy {
                        info!("🔒 Возвращаем сенсорные кнопки под управление Sentinel Station");
                        let _ = self.is_active_tx.send(true);
                        let display_tx = self.display_tx.clone();
                        tokio::spawn(async move {
                            let _ = display_tx.send(DisplayCommand::PassThroughResume).await;
                        });
                    }

                    // 3. Распознавание голосовых команд Алисы и локальных быстрых команд (Spotter)
                    let phrase_opt = if line.contains("recognizedPhrase='") {
                        Self::extract_recognized_phrase(line)
                    } else if let Some(idx) = line.find("Command phrase spotted: ") {
                        let spotted = line[idx + "Command phrase spotted: ".len()..].trim().to_lowercase();
                        if spotted.is_empty() {
                            None
                        } else {
                            Some(spotted)
                        }
                    } else {
                        None
                    };

                    if let Some(phrase) = phrase_opt {
                        let now = Instant::now();
                        // Защита от дублирования одной и той же фразы в пределах 2 секунд (например spotter + cloud ASR)
                        if phrase == last_phrase && now.duration_since(last_phrase_time) < Duration::from_millis(2000) {
                            continue;
                        }
                        last_phrase = phrase.clone();
                        last_phrase_time = now;

                        // Явные команды активации Sentinel или запуска Потока обрабатываются ВСЕГДА,
                        // даже если флаг звонка или будильника остался активным из-за бага
                        let is_wake = Self::is_activation_phrase(&phrase) || Self::is_flow_phrase(&phrase);
                        if is_wake {
                            if curr_busy {
                                info!("⚡ Явная команда активации/Потока '{}' сбрасывает флаг занятости", phrase);
                                in_call = false;
                                in_alarm_or_timer = false;
                                alarm_start_time = None;
                            }
                            self.handle_voice_phrase(&phrase);
                        } else if !curr_busy {
                            self.handle_voice_phrase(&phrase);
                        }
                    }
                }
                Err(e) => {
                    warn!("Ошибка чтения лога Quasar: {}", e);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    /// Проверка, указывает ли строка лога на входящий/исходящий звонок
    fn is_call_start(lower: &str) -> bool {
        if Self::is_call_end(lower) {
            return false;
        }

        lower.contains("incoming call")
            || lower.contains("onincomingcall")
            || lower.contains("new incoming call")
            || lower.contains("accept incoming call")
            || lower.contains("phone_calls_process_incoming_call")
            || lower.contains("tincomingcallreceivedtypedcallback")
            || lower.contains("call state changed: incoming")
            || lower.contains("call state changed: calling")
            || lower.contains("call state changed: early")
            || lower.contains("outgoing call connected")
            || lower.contains("call_incoming.led")
            || lower.contains("call_outgoing.led")
            || lower.contains("call_connecting.led")
            || lower.contains("calls_ringing_incoming")
            || lower.contains("calls_ringing_outgoing")
    }

    /// Проверка, указывает ли строка лога на окончание телефонного звонка
    fn is_call_end(lower: &str) -> bool {
        lower.contains("call ended")
            || lower.contains("current call ended")
            || lower.contains("incoming call ended")
            || lower.contains("outgoing call ended")
            || lower.contains("callishangedup")
            || lower.contains("call state changed: disconnctd")
            || lower.contains("disconnctd")
            || lower.contains("incomingcalltimeout")
            || lower.contains("phone_calls_ignore_incoming_call")
            || lower.contains("reject_incoming_call")
            || lower.contains("call_ended.led")
            || lower.contains("calls_ended")
    }

    /// Проверка, указывает ли фраза на деактивацию Sentinel
    pub fn is_deactivation_phrase(phrase: &str) -> bool {
        let p = phrase.trim();
        p == "выключи сентинел"
            || p == "отключи сентинел"
            || p == "деактивируй сентинел"
            || p == "усыпи сентинел"
            || p == "стоп сентинел"
            || p == "режим алисы"
            || p == "переключи на алису"
            || p == "верни алису"
            || p == "вернуть алису"
            || p == "выключи сентинель"
            || p == "отключи сентинель"
            || p == "режим яндекс"
            || p == "верни яндекс"
            || p == "выключи sentinel"
            || p == "отключи sentinel"
            || p == "деактивируй sentinel"
            || p == "стоп sentinel"
            || p.starts_with("выключи сентинел")
            || p.starts_with("отключи сентинел")
            || p.starts_with("верни алису")
            || p.starts_with("вернуть алису")
    }

    /// Проверка, указывает ли фраза на активацию Sentinel
    pub fn is_activation_phrase(phrase: &str) -> bool {
        let p = phrase.trim();
        p == "сентинел"
            || p == "сентинель"
            || p == "сантинел"
            || p == "сантинель"
            || p == "sentinel"
            || p == "включи сентинел"
            || p == "включить сентинел"
            || p == "запусти сентинел"
            || p == "активируй сентинел"
            || p == "разбуди сентинел"
            || p == "режим сентинел"
            || p == "переключи на сентинел"
            || p == "верни сентинел"
            || p == "вернуть сентинел"
            || p == "открой сентинел"
            || p == "включи сентинель"
            || p == "запусти сентинель"
            || p == "активируй сентинель"
            || p == "разбуди сентинель"
            || p == "режим сентинель"
            || p == "переключи на сентинель"
            || p == "верни сентинель"
            || p == "открой сентинель"
            || p == "включи sentinel"
            || p == "запусти sentinel"
            || p == "активируй sentinel"
            || p == "разбуди sentinel"
            || p == "режим sentinel"
            || p == "переключи на sentinel"
            || p == "верни sentinel"
            || p.starts_with("включи сентинел")
            || p.starts_with("включить сентинел")
            || p.starts_with("запусти сентинел")
            || p.starts_with("активируй сентинел")
            || p.starts_with("разбуди сентинел")
            || p.starts_with("верни сентинел")
            || p.starts_with("вернуть сентинел")
            || p.starts_with("открой сентинел")
            || p.starts_with("включи сентинель")
            || p.starts_with("запусти сентинель")
            || p.starts_with("верни сентинель")
            || p.starts_with("включи sentinel")
            || p.starts_with("запусти sentinel")
            || p.starts_with("верни sentinel")
    }

    /// Проверка, указывает ли фраза на запуск бесконечного Потока
    pub fn is_flow_phrase(phrase: &str) -> bool {
        let p = phrase.trim();
        p == "поток"
            || p == "мой поток"
            || p == "музыкальный поток"
            || p == "поток музыки"
            || p.starts_with("поток")
            || p.starts_with("включи поток")
            || p.starts_with("запусти поток")
            || p.starts_with("поставь поток")
            || p.starts_with("мой поток")
            || p.starts_with("включи мой поток")
            || p.starts_with("запусти мой поток")
    }

    /// Проверка, указывает ли строка лога на срабатывание будильника / таймера
    fn is_alarm_or_timer_start(lower: &str) -> bool {
        if Self::is_alarm_or_timer_end(lower) {
            return false;
        }

        lower.contains("ringing.binscled")
            || lower.contains("ringing.scled")
            || lower.contains("alarmfiredevent")
            || lower.contains("alarm event fired")
            || lower.contains("ontimerfired")
            || lower.contains("processexpiredevent")
    }

    /// Проверка, указывает ли строка лога на остановку будильника / таймера
    fn is_alarm_or_timer_end(lower: &str) -> bool {
        lower.contains("timer_stop_directive")
            || lower.contains("alarm_stop_directive")
            || lower.contains("local_audio_file_stop")
            || lower.contains("alarm was already stopped")
            || lower.contains("handlealarmstopdirective")
            || lower.contains("handletimerstopdirective")
            || lower.contains("tstopcurrentsoundalarmdirective")
            || lower.contains("timer stop directive")
            || lower.contains("alarm is still playing after timeout")
            || lower.contains("close_screen_alarm_active_directive")
    }

    /// Извлечение распознанной фразы из строки лога maind
    fn extract_recognized_phrase(line: &str) -> Option<String> {
        let pattern = "recognizedPhrase='";
        let start = line.find(pattern)? + pattern.len();
        let rest = &line[start..];
        let end = rest.find('\'')?;
        let raw = rest[..end].trim();
        if raw.is_empty() {
            None
        } else {
            let mut lower = raw.to_lowercase();
            // Срезаем префикс обращения ("алиса ", "яндекс "), если он попал в распознанную фразу
            if lower.starts_with("алиса ") {
                lower = lower["алиса ".len()..].trim().to_string();
            } else if lower.starts_with("яндекс ") {
                lower = lower["яндекс ".len()..].trim().to_string();
            }
            Some(lower)
        }
    }

    /// Обработка и маршрутизация распознанной голосовой фразы
    fn handle_voice_phrase(&self, phrase: &str) {
        info!("🗣️ Распознана голосовая фраза Алисы: '{}'", phrase);

        // 0. Деактивация / переход Sentinel в режим ожидания (Passthrough)
        if Self::is_deactivation_phrase(phrase) {
            info!("💤 Голосовая команда: ДЕАКТИВАЦИЯ Sentinel Station (переход в режим ожидания/Алисы)");
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);
            let _ = self.is_active_tx.send(false);

            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Standby).await;
                let _ = player_tx.send(PlayerCommand::Pause).await;
            });
            return;
        }

        // 0.1. Активация / пробуждение Sentinel Station
        if Self::is_activation_phrase(phrase) {
            info!("⚡ Голосовая команда: АКТИВАЦИЯ Sentinel Station (возврат управления плеером)");
            let _ = self.is_active_tx.send(true);
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);

            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Active).await;
            });
            return;
        }

        // 0.2. Запуск умного бесконечного Потока (Flow)
        if Self::is_flow_phrase(phrase) {
            let was_active = *self.is_active_tx.borrow();
            if !was_active {
                info!("⚡ Голосовая команда запуска Потока пробуждает Sentinel Station!");
                let _ = self.is_active_tx.send(true);
            }

            info!("🌊 Перехвачена голосовая команда запуска Потока: '{}'", phrase);
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);

            // Воспроизводим локальное подтверждение "Включаю Поток."
            tokio::spawn(async move {
                let sound_path = "/data/opt/sentinel-station/sounds/flow_start.wav";
                if std::path::Path::new(sound_path).exists() {
                    let _ = tokio::process::Command::new("aplay")
                        .arg("-D")
                        .arg("default")
                        .arg(sound_path)
                        .status()
                        .await;
                }
            });

            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Flow).await;
                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                let _ = player_tx.send(PlayerCommand::StartFlow).await;
            });
            return;
        }

        // Если Sentinel деактивирован, не перехватываем музыку и громкость — отдаем штатной Алисе!
        if !*self.is_active_tx.borrow() {
            info!("ℹ️ Sentinel в спящем режиме. Запрос '{}' передан штатной Алисе.", phrase);
            return;
        }

        // 1. Включение музыки / запуск трека
        // Отсекаем не относящиеся к музыке команды: таймеры, будильники, умный дом, погоду, новости и звонки
        let is_non_music = phrase.contains("таймер")
            || phrase.contains("будильник")
            || phrase.contains("напомин")
            || phrase.contains("секундомер")
            || phrase.contains("свет")
            || phrase.contains("розетк")
            || phrase.contains("чайник")
            || phrase.contains("пылесос")
            || phrase.contains("кондиционер")
            || phrase.contains("телевизор")
            || phrase.contains("люстр")
            || phrase.contains("ночник")
            || phrase.contains("яркость")
            || phrase.contains("сценари")
            || phrase.contains("радио")
            || phrase.contains("сказк")
            || phrase.contains("новости")
            || phrase.contains("погод")
            || phrase.contains("позвони")
            || phrase.contains("шум");

        let is_music_cmd = !is_non_music && (
            phrase == "включи музыку"
            || phrase == "включи трек"
            || phrase == "включи песню"
            || phrase == "включи плейлист"
            || phrase == "вруби музыку"
            || phrase == "вруби песню"
            || phrase == "вруби трек"
            || phrase == "поставь музыку"
            || phrase == "поставь песню"
            || phrase == "поставь трек"
            || phrase == "поставь плейлист"
            || phrase == "запусти музыку"
            || phrase == "запусти песню"
            || phrase == "запусти трек"
            || phrase == "играй музыку"
            || phrase == "играй"
            || phrase == "музыка"
            || phrase == "воспроизведи"
            || phrase == "продолжи"
            || phrase == "продолжить"
            || phrase == "продолжи воспроизведение"
            || phrase == "продолжай"
            || phrase == "ремикс"
            || phrase.starts_with("включи ")
            || phrase.starts_with("вруби ")
            || phrase.starts_with("поставь ")
            || phrase.starts_with("запусти ")
            || phrase.starts_with("играй ")
            || phrase.starts_with("сыграй ")
            || phrase.starts_with("ремикс")
            || phrase.starts_with("музыку ")
            || phrase.starts_with("трек ")
            || phrase.starts_with("песню ")
            || phrase.starts_with("плейлист ")
        );

        if is_music_cmd {
            info!("🎵 Перехвачена команда воспроизведения: '{}'! Запуск Sentinel Music", phrase);

            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();

            // Немедленно останавливаем штатную Алису и запускаем Sentinel Music без задержек,
            // чтобы родной плеер Алисы не успел запуститься и наложиться на музыку Sentinel.
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Play).await;
                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                let _ = player_tx.send(PlayerCommand::Play).await;

                // Повторный сброс через 800мс на случай отложенного ответа облака Яндекса
                tokio::time::sleep(Duration::from_millis(800)).await;
                Self::stop_alice_playback();
            });
            return;
        }

        // 2. Пауза / Стоп
        if phrase == "пауза"
            || phrase == "стоп"
            || phrase == "останови"
            || phrase == "останови музыку"
            || phrase == "выключи музыку"
            || phrase == "хватит"
            || phrase == "замолчи"
            || phrase == "тишина"
        {
            info!("⏸️ Перехвачена команда паузы: '{}'", phrase);
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Pause).await;
                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                let _ = player_tx.send(PlayerCommand::Pause).await;
            });
            return;
        }

        // 3. Следующий трек
        if phrase == "следующий трек"
            || phrase == "следующая песня"
            || phrase == "следующий"
            || phrase == "следующая"
            || phrase == "дальше"
            || phrase == "переключи"
            || phrase == "переключи трек"
            || phrase == "вперед"
        {
            info!("⏭️ Перехвачена команда перехода к следующему треку: '{}'", phrase);
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::NextTrack).await;
                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                let _ = player_tx.send(PlayerCommand::Next).await;
                tokio::time::sleep(Duration::from_millis(800)).await;
                Self::stop_alice_playback();
            });
            return;
        }

        // 4. Предыдущий трек
        if phrase == "предыдущий трек"
            || phrase == "предыдущая песня"
            || phrase == "предыдущий"
            || phrase == "предыдущая"
            || phrase == "назад"
        {
            info!("⏮️ Перехвачена команда перехода к предыдущему треку: '{}'", phrase);
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::PrevTrack).await;
                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                let _ = player_tx.send(PlayerCommand::Prev).await;
                tokio::time::sleep(Duration::from_millis(800)).await;
                Self::stop_alice_playback();
            });
            return;
        }

        // 5. Управление громкостью
        if phrase == "громче"
            || phrase == "прибавь"
            || phrase == "прибавь громкость"
            || phrase == "прибавить"
            || phrase == "сделай громче"
            || phrase == "добавь звук"
            || phrase == "добавь громкости"
        {
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);
            let current_vol = self.state_rx.borrow().volume;
            let new_vol = (current_vol + 0.10).min(1.0);
            info!("🔊 Перехвачена команда 'громче': {:.0}% -> {:.0}%", current_vol * 100.0, new_vol * 100.0);
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Volume(new_vol)).await;
                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                let _ = player_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
            });
            return;
        }

        if phrase == "тише"
            || phrase == "убавь"
            || phrase == "убавь громкость"
            || phrase == "убавить"
            || phrase == "сделай тише"
            || phrase == "убавь звук"
        {
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);
            let current_vol = self.state_rx.borrow().volume;
            let new_vol = (current_vol - 0.10).max(0.0);
            info!("🔉 Перехвачена команда 'тише': {:.0}% -> {:.0}%", current_vol * 100.0, new_vol * 100.0);
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Volume(new_vol)).await;
                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                let _ = player_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
            });
            return;
        }

        // 6. Перемешивание (Shuffle)
        if phrase == "перемешай"
            || phrase == "перемешать"
            || phrase == "перемешай треки"
            || phrase == "перемешать треки"
            || phrase == "перемешай музыку"
            || phrase == "перемешать музыку"
            || phrase == "перемешай плейлист"
            || phrase == "перемешать плейлист"
            || phrase == "включи перемешивание"
            || phrase == "случайный порядок"
            || phrase == "в случайном порядке"
            || phrase == "вперемешку"
            || phrase == "шафл"
            || phrase == "шаффл"
            || phrase == "включи шафл"
            || phrase == "включи шаффл"
            || phrase.starts_with("перемешай")
            || phrase.starts_with("перемешать")
        {
            info!("🔀 Перехвачена команда перемешивания: '{}'", phrase);
            Self::stop_alice_playback();
            self.is_ducked.store(false, Ordering::SeqCst);
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = player_tx.send(PlayerCommand::Duck { ducked: false }).await;
                let _ = display_tx.send(DisplayCommand::PassThroughResume).await;
                let _ = player_tx.send(PlayerCommand::ToggleShuffle).await;
            });
            return;
        }

        info!("ℹ️ Голосовой запрос Алисы пропущен без перехвата (не относится к музыке): '{}'", phrase);
    }
}
