use std::fs::File;
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

use crate::connect::protocol::{PlayerCommand, PlayerState};
use crate::visualizer::DisplayCommand;

const EV_KEY: u16 = 0x01;

// ioctl код для EVIOCGRAB: _IOW('E', 0x90, int) -> 0x40044590
// Эксклюзивно перехватывает все нажатия кнопок, блокируя их получение Алисой (platformd)
const EVIOCGRAB: libc::c_ulong = 0x40044590;

const KEY_VOLUMEDOWN: u16 = 114;
const KEY_VOLUMEUP: u16 = 115;
const KEY_PLAYPAUSE: u16 = 164;
const KEY_NEXTSONG: u16 = 163;
const KEY_PREVIOUSSONG: u16 = 165;
const KEY_PAUSE: u16 = 119;
const KEY_PLAY: u16 = 207;
const KEY_VOICECOMMAND: u16 = 582; // Центральная кнопка (Алиса / Пауза) на AW9610X

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TouchPad {
    Left = 0,    // KEY_VOLUMEDOWN (114) -> '-'
    Center = 1,  // KEY_VOICECOMMAND (582), KEY_PLAYPAUSE (164), etc. -> 'o'
    Right = 2,   // KEY_VOLUMEUP (115) -> '+'
}

impl TouchPad {
    fn from_code(code: u16) -> Option<Self> {
        match code {
            KEY_VOLUMEDOWN => Some(TouchPad::Left),
            KEY_VOICECOMMAND | KEY_PLAYPAUSE | KEY_PAUSE | KEY_PLAY => Some(TouchPad::Center),
            KEY_VOLUMEUP => Some(TouchPad::Right),
            _ => None,
        }
    }

    fn pos(&self) -> i32 {
        *self as i32
    }
}

pub struct HardwareButtonController {
    device_path: String,
    cmd_tx: mpsc::Sender<PlayerCommand>,
    state_rx: watch::Receiver<PlayerState>,
    display_tx: mpsc::Sender<DisplayCommand>,
}

impl HardwareButtonController {
    pub fn new(
        device_path: impl Into<String>,
        cmd_tx: mpsc::Sender<PlayerCommand>,
        state_rx: watch::Receiver<PlayerState>,
        display_tx: mpsc::Sender<DisplayCommand>,
    ) -> Self {
        Self {
            device_path: device_path.into(),
            cmd_tx,
            state_rx,
            display_tx,
        }
    }

    /// Запуск фонового контроллера сенсорных кнопок
    pub fn start(self) {
        let path_str = self.device_path.clone();
        if !Path::new(&path_str).exists() {
            warn!("Устройство ввода {} не найдено. Аппаратные кнопки отключены.", path_str);
            return;
        }

        let (raw_tx, mut raw_rx) = mpsc::channel::<(u16, i32)>(32);

        // 1. Поток прямого чтения из /dev/input/event0
        let thread_path = path_str.clone();
        std::thread::Builder::new()
            .name("sentinel-evdev".to_string())
            .spawn(move || {
                let mut file = match File::open(&thread_path) {
                    Ok(f) => f,
                    Err(e) => {
                        error!("Не удалось открыть устройство ввода {}: {}", thread_path, e);
                        return;
                    }
                };

                // Эксклюзивный захват устройства ввода через ioctl(EVIOCGRAB, 1).
                // Блокирует передачу событий сенсорных кнопок сторонним процессам (platformd / Алиса).
                let fd = file.as_raw_fd();
                let grab_ret = unsafe { libc::ioctl(fd, EVIOCGRAB, 1) };
                if grab_ret == 0 {
                    info!("🔒 Сенсорная панель успешно перехвачена монопольно (EVIOCGRAB). Нажатия кнопок изолированы от Алисы!");
                } else {
                    warn!("Не удалось монопольно захватить {}: код ошибки {}, {}", thread_path, grab_ret, std::io::Error::last_os_error());
                }

                info!("Чтение сенсорных кнопок запущено на {}", thread_path);
                let mut buf = [0u8; 96];

                loop {
                    let n = match file.read(&mut buf) {
                        Ok(0) => {
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        Ok(n) => n,
                        Err(e) => {
                            error!("Ошибка чтения evdev {}: {}", thread_path, e);
                            std::thread::sleep(Duration::from_millis(1000));
                            continue;
                        }
                    };

                    let event_size = if n % 24 == 0 && n % 16 != 0 {
                        24
                    } else if n % 16 == 0 {
                        16
                    } else if n % 24 == 0 {
                        24
                    } else {
                        continue;
                    };

                    let mut offset = 0;
                    while offset + event_size <= n {
                        let chunk = &buf[offset..offset + event_size];
                        let (type_, code, value) = if event_size == 16 {
                            let t = u16::from_ne_bytes([chunk[8], chunk[9]]);
                            let c = u16::from_ne_bytes([chunk[10], chunk[11]]);
                            let v = i32::from_ne_bytes([chunk[12], chunk[13], chunk[14], chunk[15]]);
                            (t, c, v)
                        } else {
                            let t = u16::from_ne_bytes([chunk[16], chunk[17]]);
                            let c = u16::from_ne_bytes([chunk[18], chunk[19]]);
                            let v = i32::from_ne_bytes([chunk[20], chunk[21], chunk[22], chunk[23]]);
                            (t, c, v)
                        };

                        if type_ == EV_KEY {
                            if raw_tx.blocking_send((code, value)).is_err() {
                                return;
                            }
                        }
                        offset += event_size;
                    }
                }
            })
            .expect("Не удалось запустить поток чтения evdev");

        // 2. Асинхронный автомат с подавлением емкостного дребезга (ClickLimiter & SwipeResolver)
        let cmd_tx = self.cmd_tx;
        let state_rx = self.state_rx;
        let display_tx = self.display_tx;

        tokio::spawn(async move {
            // Время нажатия каждого сенсора (None если отпущен)
            let mut pressed_at: [Option<Instant>; 3] = [None; 3];
            // Время последнего подтвержденного клика по каждому сенсору
            let mut last_click_at: [Option<Instant>; 3] = [None; 3];
            // История недавних нажатий для распознавания свайпа
            let mut recent_presses: Vec<(TouchPad, Instant)> = Vec::new();
            // Период охлаждения после свайпа (подавляет весь дребезг и отпускания)
            let mut cooldown_until: Option<Instant> = None;

            // Константы защиты от аппаратного дребезга емкостной панели
            const MIN_PRESS_DURATION: Duration = Duration::from_millis(40); // Минимум удержания для отсечки микро-всплесков
            const MIN_CLICK_INTERVAL: Duration = Duration::from_millis(220); // Защита от бесконечного пролистывания громкости
            const SWIPE_WINDOW: Duration = Duration::from_millis(300); // Окно связывания нажатий в свайп
            const SWIPE_COOLDOWN: Duration = Duration::from_millis(450); // Блокировка после свайпа

            while let Some((code, value)) = raw_rx.recv().await {
                debug!("Событие кнопки: code={}, value={}", code, value);

                // Прямые аппаратные коды переключения трека
                if code == KEY_NEXTSONG && value == 0 {
                    info!("⏭️ Аппаратная кнопка: Следующий трек");
                    let _ = display_tx.send(DisplayCommand::NextTrack).await;
                    let _ = cmd_tx.send(PlayerCommand::Next).await;
                    continue;
                } else if code == KEY_PREVIOUSSONG && value == 0 {
                    info!("⏮️ Аппаратная кнопка: Предыдущий трек");
                    let _ = display_tx.send(DisplayCommand::PrevTrack).await;
                    let _ = cmd_tx.send(PlayerCommand::Prev).await;
                    continue;
                }

                let pad = match TouchPad::from_code(code) {
                    Some(p) => p,
                    None => continue,
                };

                let now = Instant::now();
                let pad_idx = pad.pos() as usize;

                // 1. Проверка периода охлаждения после свайпа
                if let Some(cooldown) = cooldown_until {
                    if now < cooldown {
                        if value == 0 {
                            pressed_at[pad_idx] = None;
                        }
                        continue;
                    } else {
                        cooldown_until = None;
                    }
                }

                if value == 1 {
                    // НАЖАТИЕ СЕНСОРА
                    // Игнорируем повторный value=1, если кнопка уже удерживается
                    if pressed_at[pad_idx].is_some() {
                        continue;
                    }

                    // Проверяем минимальный интервал между повторными кликами одной кнопки
                    if let Some(last_click) = last_click_at[pad_idx] {
                        if now.duration_since(last_click) < MIN_CLICK_INTERVAL {
                            debug!("Отсечен емкостной дребезг повторного нажатия {:?}: прошло < 220мс", pad);
                            continue;
                        }
                    }

                    // Очищаем старые нажатия (> 300 мс)
                    recent_presses.retain(|(_, t)| now.duration_since(*t) <= SWIPE_WINDOW);

                    // Проверяем, образует ли это нажатие свайп с другим сенсором:
                    let mut swipe_dir: Option<i32> = None;

                    // 1) Другой сенсор удерживается прямо сейчас
                    for (idx, at) in pressed_at.iter().enumerate() {
                        if at.is_some() && idx != pad_idx {
                            swipe_dir = Some(pad.pos() - idx as i32);
                            break;
                        }
                    }

                    // 2) Другой сенсор был нажат совсем недавно (скольжение пальца)
                    if swipe_dir.is_none() {
                        if let Some((prev_pad, _)) = recent_presses.last() {
                            if *prev_pad != pad {
                                swipe_dir = Some(pad.pos() - prev_pad.pos());
                            }
                        }
                    }

                    if let Some(dir) = swipe_dir {
                        // СВАЙП ОБНАРУЖЕН!
                        pressed_at = [None; 3];
                        recent_presses.clear();
                        cooldown_until = Some(now + SWIPE_COOLDOWN);

                        if dir > 0 {
                            info!("⏭️ Распознан жест: свайп слева направо (->). Следующий трек");
                            let _ = display_tx.send(DisplayCommand::NextTrack).await;
                            let _ = cmd_tx.send(PlayerCommand::Next).await;
                        } else {
                            info!("⏮️ Распознан жест: свайп справа налево (<-). Предыдущий трек");
                            let _ = display_tx.send(DisplayCommand::PrevTrack).await;
                            let _ = cmd_tx.send(PlayerCommand::Prev).await;
                        }
                    } else {
                        // Одиночное нажатие: фиксируем начало касания
                        pressed_at[pad_idx] = Some(now);
                        recent_presses.push((pad, now));
                    }
                } else if value == 0 {
                    // ОТПУСКАНИЕ СЕНСОРА
                    let press_start = match pressed_at[pad_idx].take() {
                        Some(t) => t,
                        None => continue, // Нажатие было отсечено как дребезг
                    };

                    let press_duration = now.duration_since(press_start);

                    // Защита от микро-всплесков (касание короче 40 мс)
                    if press_duration < MIN_PRESS_DURATION {
                        debug!("Отсечено ложное микро-касание {:?} длительностью {:?}", pad, press_duration);
                        continue;
                    }

                    // Защита от слишком быстрых кликов
                    if let Some(last_click) = last_click_at[pad_idx] {
                        if now.duration_since(last_click) < MIN_CLICK_INTERVAL {
                            debug!("Отсечено повторное отпускание дребезга {:?}", pad);
                            continue;
                        }
                    }

                    // Если прямо сейчас нажат другой сенсор — это часть свайпа
                    if pressed_at.iter().any(|p| p.is_some()) {
                        continue;
                    }

                    // ОДИНОЧНЫЙ КЛИК ПОДТВЕРЖДЕН!
                    last_click_at[pad_idx] = Some(now);

                    match pad {
                        TouchPad::Left => {
                            let cur = state_rx.borrow().volume;
                            let new_vol = ((cur - 0.05) * 100.0).round() / 100.0;
                            let new_vol = new_vol.max(0.0);
                            info!("🔉 Сенсорная кнопка '-': Громкость {:.0}%", new_vol * 100.0);
                            let _ = display_tx.send(DisplayCommand::Volume(new_vol)).await;
                            let _ = cmd_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
                        }
                        TouchPad::Right => {
                            let cur = state_rx.borrow().volume;
                            let new_vol = ((cur + 0.05) * 100.0).round() / 100.0;
                            let new_vol = new_vol.min(1.0);
                            info!("🔊 Сенсорная кнопка '+': Громкость {:.0}%", new_vol * 100.0);
                            let _ = display_tx.send(DisplayCommand::Volume(new_vol)).await;
                            let _ = cmd_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
                        }
                        TouchPad::Center => {
                            info!("🔘 Сенсорная кнопка: Пауза / Воспроизведение");
                            let _ = cmd_tx.send(PlayerCommand::TogglePlay).await;
                        }
                    }
                }
            }
        });
    }
}
