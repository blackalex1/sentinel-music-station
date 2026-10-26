use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use crate::connect::protocol::{PlayerCommand, PlayerState};
use crate::visualizer::DisplayCommand;

/// Локальный перехватчик голосовых команд Алисы через журнал событий Quasar (maind)
pub struct VoiceInterceptor {
    player_tx: mpsc::Sender<PlayerCommand>,
    state_rx: watch::Receiver<PlayerState>,
    display_tx: mpsc::Sender<DisplayCommand>,
    log_path: PathBuf,
}

impl VoiceInterceptor {
    pub fn new(
        player_tx: mpsc::Sender<PlayerCommand>,
        state_rx: watch::Receiver<PlayerState>,
        display_tx: mpsc::Sender<DisplayCommand>,
    ) -> Self {
        Self {
            player_tx,
            state_rx,
            display_tx,
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

    /// Блокировка (заморозка) воспроизведения Яндекс.Музыки через SIGSTOP к mediad
    pub fn suspend_yandex_music() {
        if let Some(pid) = Self::find_mediad_pid() {
            unsafe {
                libc::kill(pid as i32, libc::SIGSTOP);
            }
            info!("🔇 Яндекс.Музыка заблокирована (mediad PID {} заморожен через SIGSTOP)", pid);
        }
    }

    /// Разморозка mediad через SIGCONT (если понадобится штатное воспроизведение)
    #[allow(dead_code)]
    pub fn resume_yandex_music() {
        if let Some(pid) = Self::find_mediad_pid() {
            unsafe {
                libc::kill(pid as i32, libc::SIGCONT);
            }
            info!("🔊 mediad PID {} разморожен (SIGCONT)", pid);
        }
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
        let raw = &rest[..end].trim();
        if raw.is_empty() {
            None
        } else {
            Some(raw.to_lowercase())
        }
    }

    /// Обработка и маршрутизация распознанной голосовой фразы
    fn handle_voice_phrase(&self, phrase: &str) {
        info!("🗣️ Распознана голосовая фраза Алисы: '{}'", phrase);

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
            // после чего плавно и без наложения стартует трек из Sentinel Music
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(2200)).await;
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
