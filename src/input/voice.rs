use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::connect::protocol::{PlayerCommand, PlayerState};
use crate::visualizer::DisplayCommand;

/// Локальный перехватчик голосовых команд Алисы через журнал событий Quasar (maind)
pub struct VoiceInterceptor {
    player_tx: mpsc::Sender<PlayerCommand>,
    state_rx: watch::Receiver<PlayerState>,
    display_tx: mpsc::Sender<DisplayCommand>,
    is_active_tx: watch::Sender<bool>,
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
                .arg("(sleep 0.1; echo o; echo c; sleep 0.2; echo x) | LD_LIBRARY_PATH=/system/vendor/quasar/libs /system/vendor/quasar/maind --cli >/dev/null 2>&1")
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

        // Перемещаемся в конец файла, чтобы обрабатывать только новые голосовые события
        let _ = file.seek(SeekFrom::End(0));
        let mut reader = BufReader::new(file);

        let mut last_phrase = String::new();
        let mut last_phrase_time = Instant::now();

        let mut line_buffer = String::new();

        loop {
            line_buffer.clear();
            match reader.read_line(&mut line_buffer) {
                Ok(0) => {
                    // Конец файла (EOF): проверяем ротацию файла (если размер уменьшился)
                    if let Ok(meta) = fs::metadata(&self.log_path) {
                        if let Ok(pos) = reader.stream_position() {
                            if meta.len() < pos {
                                // Файл ротировался, переоткрываем
                                if let Ok(new_file) = File::open(&self.log_path) {
                                    reader = BufReader::new(new_file);
                                }
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Ok(_) => {
                    let line = line_buffer.trim();
                    if line.contains("recognizedPhrase='") {
                        if let Some(phrase) = Self::extract_recognized_phrase(line) {
                            let now = Instant::now();
                            // Защита от дублирования одной и той же фразы в пределах 1 секунды
                            if phrase == last_phrase && now.duration_since(last_phrase_time) < Duration::from_millis(1000) {
                                continue;
                            }
                            last_phrase = phrase.clone();
                            last_phrase_time = now;

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
        if phrase == "выключи сентинел"
            || phrase == "отключи сентинел"
            || phrase == "деактивируй сентинел"
            || phrase == "усыпи сентинел"
            || phrase == "стоп сентинел"
            || phrase == "режим алисы"
            || phrase == "переключи на алису"
            || phrase == "верни алису"
            || phrase == "выключи sentinel"
            || phrase == "отключи sentinel"
            || phrase == "деактивируй sentinel"
            || phrase == "стоп sentinel"
            || phrase == "режим яндекс"
            || phrase == "верни яндекс"
        {
            info!("💤 Голосовая команда: ДЕАКТИВАЦИЯ Sentinel Station (переход в режим ожидания/Алисы)");
            Self::stop_alice_playback();
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
        if phrase == "включи сентинел"
            || phrase == "запусти сентинел"
            || phrase == "активируй сентинел"
            || phrase == "разбуди сентинел"
            || phrase == "режим сентинел"
            || phrase == "переключи на сентинел"
            || phrase == "верни сентинел"
            || phrase == "сентинел"
            || phrase == "включи sentinel"
            || phrase == "запусти sentinel"
            || phrase == "активируй sentinel"
            || phrase == "разбуди sentinel"
            || phrase == "режим sentinel"
            || phrase == "переключи на sentinel"
            || phrase == "верни sentinel"
            || phrase == "sentinel"
        {
            info!("⚡ Голосовая команда: АКТИВАЦИЯ Sentinel Station (возврат управления плеером)");
            let _ = self.is_active_tx.send(true);
            Self::stop_alice_playback();

            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Active).await;
            });
            return;
        }

        // 0.2. Запуск умного бесконечного Потока (Flow)
        let is_flow_cmd = phrase == "поток"
            || phrase == "мой поток"
            || phrase == "музыкальный поток"
            || phrase == "поток музыки"
            || phrase.starts_with("включи поток")
            || phrase.starts_with("запусти поток")
            || phrase.starts_with("мой поток")
            || phrase.starts_with("включи мой поток")
            || phrase.starts_with("запусти мой поток");

        if is_flow_cmd {
            let was_active = *self.is_active_tx.borrow();
            if !was_active {
                info!("⚡ Голосовая команда запуска Потока пробуждает Sentinel Station!");
                let _ = self.is_active_tx.send(true);
            }

            info!("🌊 Перехвачена голосовая команда запуска Потока: '{}'", phrase);
            Self::stop_alice_playback();

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
        if phrase == "включи музыку"
            || phrase == "включи трек"
            || phrase == "включи песню"
            || phrase == "включи плейлист"
            || phrase == "поставь музыку"
            || phrase == "поставь песню"
            || phrase == "поставь трек"
            || phrase == "запусти музыку"
            || phrase == "играй музыку"
            || phrase == "музыка"
            || phrase == "продолжи"
            || phrase == "продолжить"
            || phrase == "продолжи воспроизведение"
            || phrase.starts_with("включи ")
            || phrase.starts_with("поставь ")
            || phrase.starts_with("запусти ")
            || phrase.starts_with("играй ")
        {
            info!("🎵 Перехвачена команда воспроизведения: '{}'! Запуск Sentinel Music", phrase);

            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();

            // Даем Алисе 2.2 секунды, чтобы она полностью закончила фразу ("Включаю музыку"),
            // после чего отменяем воспроизведение в Яндекс.Музыке и запускаем трек в Sentinel
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(2200)).await;
                Self::stop_alice_playback();
                let _ = display_tx.send(DisplayCommand::Play).await;
                let _ = player_tx.send(PlayerCommand::Play).await;
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
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Pause).await;
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
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::NextTrack).await;
                let _ = player_tx.send(PlayerCommand::Next).await;
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
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::PrevTrack).await;
                let _ = player_tx.send(PlayerCommand::Prev).await;
            });
            return;
        }

        // 5. Управление громкостью
        if phrase == "громче" || phrase == "прибавь" || phrase == "прибавь громкость" {
            let current_vol = self.state_rx.borrow().volume;
            let new_vol = (current_vol + 0.10).min(1.0);
            info!("🔊 Перехвачена команда 'громче': {:.0}% -> {:.0}%", current_vol * 100.0, new_vol * 100.0);
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Volume(new_vol)).await;
                let _ = player_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
            });
            return;
        }

        if phrase == "тише" || phrase == "убавь" || phrase == "убавь громкость" {
            let current_vol = self.state_rx.borrow().volume;
            let new_vol = (current_vol - 0.10).max(0.0);
            info!("🔉 Перехвачена команда 'тише': {:.0}% -> {:.0}%", current_vol * 100.0, new_vol * 100.0);
            let player_tx = self.player_tx.clone();
            let display_tx = self.display_tx.clone();
            tokio::spawn(async move {
                let _ = display_tx.send(DisplayCommand::Volume(new_vol)).await;
                let _ = player_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
            });
            return;
        }

        // 6. Перемешивание (Shuffle)
        if phrase == "перемешай"
            || phrase == "перемешай треки"
            || phrase == "перемешай музыку"
            || phrase == "включи перемешивание"
            || phrase == "случайный порядок"
            || phrase == "вперемешку"
            || phrase == "шафл"
            || phrase == "шаффл"
        {
            info!("🔀 Перехвачена команда перемешивания: '{}'", phrase);
            let player_tx = self.player_tx.clone();
            tokio::spawn(async move {
                let _ = player_tx.send(PlayerCommand::ToggleShuffle).await;
            });
            return;
        }

        info!("ℹ️ Голосовой запрос Алисы пропущен без перехвата (не относится к музыке): '{}'", phrase);
    }
}
