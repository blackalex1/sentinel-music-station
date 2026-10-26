use std::path::PathBuf;
use tracing::{debug, info};

/// Стиль анимации переключения треков
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackAnimStyle {
    Curtain,
    Tape,
    Slot,
    Chevron,
    Random,
}

impl TrackAnimStyle {
    pub fn from_str_opt(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "curtain" | "scanner" | "1" => Self::Curtain,
            "tape" | "cassette" | "4" => Self::Tape,
            "slot" | "roll" | "5" => Self::Slot,
            "chevron" | "arrow" | "arrows" | "2" => Self::Chevron,
            _ => Self::Random,
        }
    }
}

/// Сегменты одного 7-сегментного индикатора
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DigitSegments {
    pub a: bool, // верхний горизонтальный
    pub b: bool, // верхний правый вертикальный
    pub c: bool, // нижний правый вертикальный
    pub d: bool, // нижний горизонтальный
    pub e: bool, // нижний левый вертикальный
    pub f: bool, // верхний левый вертикальный
    pub g: bool, // средний горизонтальный
}

/// Кадр отображения на 4-разрядном экране Яндекс Станции Мини 2
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DisplayFrame {
    pub digits: [DigitSegments; 4],
    pub colon_upper: bool, // точка верхняя двоеточия (PLUS1)
    pub colon_lower: bool, // точка нижняя двоеточия (PLUS2)
    pub minus: bool,       // средняя черта между 2 и 3 цифрами
    pub degree: bool,      // значок градуса
}

/// Преобразование символа в маску сегментов 7-сегментного индикатора
pub fn char_to_segments(ch: char) -> DigitSegments {
    match ch {
        '0' => DigitSegments { a: true, b: true, c: true, d: true, e: true, f: true, g: false },
        '1' => DigitSegments { a: false, b: true, c: true, d: false, e: false, f: false, g: false },
        '2' => DigitSegments { a: true, b: true, c: false, d: true, e: true, f: false, g: true },
        '3' => DigitSegments { a: true, b: true, c: true, d: true, e: false, f: false, g: true },
        '4' => DigitSegments { a: false, b: true, c: true, d: false, e: false, f: true, g: true },
        '5' => DigitSegments { a: true, b: false, c: true, d: true, e: false, f: true, g: true },
        '6' => DigitSegments { a: true, b: false, c: true, d: true, e: true, f: true, g: true },
        '7' => DigitSegments { a: true, b: true, c: true, d: false, e: false, f: false, g: false },
        '8' => DigitSegments { a: true, b: true, c: true, d: true, e: true, f: true, g: true },
        '9' => DigitSegments { a: true, b: true, c: true, d: true, e: false, f: true, g: true },
        '-' => DigitSegments { a: false, b: false, c: false, d: false, e: false, f: false, g: true },
        '_' => DigitSegments { a: false, b: false, c: false, d: true, e: false, f: false, g: false },
        '>' => DigitSegments { a: false, b: true, c: true, d: false, e: false, f: false, g: true }, // Шеврон вправо
        '<' => DigitSegments { a: false, b: false, c: false, d: false, e: true, f: true, g: true }, // Шеврон влево
        '=' => DigitSegments { a: false, b: false, c: false, d: true, e: false, f: false, g: true }, // Две полосы (=)
        '|' => DigitSegments { a: false, b: false, c: false, d: false, e: true, f: true, g: false }, // Вертикальная полоса левая
        'P' | 'p' => DigitSegments { a: true, b: true, c: false, d: false, e: true, f: true, g: true },
        'A' | 'a' => DigitSegments { a: true, b: true, c: true, d: false, e: true, f: true, g: true },
        'U' | 'u' => DigitSegments { a: false, b: true, c: true, d: true, e: true, f: true, g: false },
        'S' | 's' => DigitSegments { a: true, b: false, c: true, d: true, e: false, f: true, g: true },
        'E' | 'e' => DigitSegments { a: true, b: false, c: false, d: true, e: true, f: true, g: true },
        'O' | 'o' => DigitSegments { a: true, b: true, c: true, d: true, e: true, f: true, g: false },
        'F' | 'f' => DigitSegments { a: true, b: false, c: false, d: false, e: true, f: true, g: true },
        'N' | 'n' => DigitSegments { a: false, b: false, c: true, d: false, e: true, f: false, g: true },
        'T' | 't' => DigitSegments { a: false, b: false, c: false, d: true, e: true, f: true, g: true },
        'L' | 'l' => DigitSegments { a: false, b: false, c: false, d: true, e: true, f: true, g: false },
        'I' | 'i' => DigitSegments { a: false, b: true, c: true, d: false, e: false, f: false, g: false },
        'C' | 'c' => DigitSegments { a: true, b: false, c: false, d: true, e: true, f: true, g: false },
        'D' | 'd' => DigitSegments { a: false, b: true, c: true, d: true, e: true, f: false, g: true },
        'W' | 'w' => DigitSegments { a: false, b: false, c: true, d: true, e: true, f: false, g: true },
        'V' | 'v' => DigitSegments { a: false, b: false, c: true, d: true, e: true, f: false, g: false },
        'B' | 'b' => DigitSegments { a: false, b: false, c: true, d: true, e: true, f: true, g: true },
        'R' | 'r' => DigitSegments { a: false, b: false, c: false, d: false, e: true, f: false, g: true },
        ' ' => DigitSegments::default(),
        _ => DigitSegments::default(),
    }
}

impl DisplayFrame {
    /// Текстовая надпись длиной до 4 символов (например " OFF", "  ON", "SENT")
    pub fn text(s: &str) -> Self {
        let mut frame = Self::default();
        let chars: Vec<char> = s.chars().collect();
        for i in 0..4 {
            if i < chars.len() {
                frame.digits[i] = char_to_segments(chars[i]);
            }
        }
        frame
    }

    /// Бегущая строка: окно из 4 символов со смещением `window`
    pub fn marquee(full_text: &str, window: usize) -> Self {
        let chars: Vec<char> = full_text.chars().collect();
        let mut frame = Self::default();
        for i in 0..4 {
            let char_idx = window + i;
            if char_idx < chars.len() {
                frame.digits[i] = char_to_segments(chars[char_idx]);
            }
        }
        frame
    }

    /// Живые глазки: открыты (разряды 1 и 2)
    pub fn open_eyes() -> Self {
        let mut frame = Self::default();
        let eye = DigitSegments { a: true, b: true, c: true, d: true, e: true, f: true, g: false };
        frame.digits[1] = eye;
        frame.digits[2] = eye;
        frame
    }

    /// Живые глазки: моргание (горизонтальная полоска по центру)
    pub fn blink_eyes() -> Self {
        let mut frame = Self::default();
        frame.digits[1].g = true;
        frame.digits[2].g = true;
        frame.minus = true;
        frame
    }

    /// Живые глазки: взгляд влево
    pub fn look_left() -> Self {
        let mut frame = Self::default();
        let left_eye = DigitSegments { a: true, b: false, c: false, d: true, e: true, f: true, g: false };
        frame.digits[1] = left_eye;
        frame.digits[2] = left_eye;
        frame
    }

    /// Живые глазки: взгляд вправо
    pub fn look_right() -> Self {
        let mut frame = Self::default();
        let right_eye = DigitSegments { a: true, b: true, c: true, d: true, e: false, f: false, g: false };
        frame.digits[1] = right_eye;
        frame.digits[2] = right_eye;
        frame
    }

    /// Живые глазки: подмигивание (левый открыт, правый прищурен)
    pub fn wink_eyes() -> Self {
        let mut frame = Self::default();
        frame.digits[1] = DigitSegments { a: true, b: true, c: true, d: true, e: true, f: true, g: false };
        frame.digits[2].g = true;
        frame
    }

    /// Живые глазки: довольные дуги ^^
    pub fn happy_eyes() -> Self {
        let mut frame = Self::default();
        let arch = DigitSegments { a: true, b: true, c: false, d: false, e: false, f: true, g: true };
        frame.digits[1] = arch;
        frame.digits[2] = arch;
        frame
    }

    /// Засыпающие глазки (стадии 0..3: открыты -> опущены -> щелочки -> глубокий сон)
    pub fn sleepy_eyes(stage: usize) -> Self {
        let mut frame = Self::default();
        match stage {
            0 => {
                let eye = DigitSegments { a: true, b: true, c: true, d: true, e: true, f: true, g: false };
                frame.digits[1] = eye;
                frame.digits[2] = eye;
            }
            1 => {
                // Сонные опущенные веки (нижний полубокс o o)
                let lower_box = DigitSegments { a: false, b: false, c: true, d: true, e: true, f: false, g: true };
                frame.digits[1] = lower_box;
                frame.digits[2] = lower_box;
            }
            2 => {
                // Полузакрытые щелочки (- - -)
                frame.digits[1].g = true;
                frame.digits[2].g = true;
                frame.minus = true;
            }
            _ => {
                // Полностью уснули (нижние ресницы _ _)
                frame.digits[1].d = true;
                frame.digits[2].d = true;
            }
        }
        frame
    }
    /// Значок паузы: две центрированные параллельные вертикальные полосы ||
    pub fn pause() -> Self {
        let mut frame = Self::default();
        // Разряд 2: правая вертикаль (B2, C2)
        // Разряд 3: левая вертикаль (E3, F3)
        frame.digits[1].b = true;
        frame.digits[1].c = true;
        frame.digits[2].e = true;
        frame.digits[2].f = true;
        frame
    }

    /// Графический эквалайзер спектра (4 частотные полосы)
    pub fn spectrum(bands: &[f32]) -> Self {
        let mut frame = Self::default();
        for col in 0..4 {
            let val = if col < bands.len() { bands[col] } else { 0.0 };
            // Нижняя черта: сэмплы присутствуют (минимальный уровень сигнала)
            if val >= 0.08 {
                frame.digits[col].d = true;
            }
            // Средняя черта: средний уровень энергии полосы
            if val >= 0.32 {
                frame.digits[col].g = true;
            }
            // Верхняя черта: пик ритмического удара
            if val >= 0.62 {
                frame.digits[col].a = true;
            }
        }
        frame
    }

    /// Отображение уровня громкости (например " 70 " или " 5  ")
    pub fn volume(pct: u32) -> Self {
        let mut frame = Self::default();
        let text = if pct >= 100 {
            " 100".to_string()
        } else if pct >= 10 {
            format!(" {:02} ", pct)
        } else {
            format!("  {} ", pct)
        };

        let chars: Vec<char> = text.chars().collect();
        for (i, &ch) in chars.iter().take(4).enumerate() {
            frame.digits[i] = char_to_segments(ch);
        }
        frame
    }

    /// 1. Шторка / Сканер (Curtain Scanner: скольжение по 8 вертикальным позициям)
    pub fn curtain_track(step: usize, is_next: bool) -> Self {
        let mut frame = Self::default();
        let pos = if is_next { step } else { 7usize.saturating_sub(step) };
        if pos < 8 {
            let d = pos / 2;
            let is_right = (pos % 2) == 1;
            if is_right {
                frame.digits[d].b = true;
                frame.digits[d].c = true;
            } else {
                frame.digits[d].f = true;
                frame.digits[d].e = true;
            }
            // Хвост затухания на предыдущем ребре
            if is_next && pos > 0 {
                let pd = (pos - 1) / 2;
                if ((pos - 1) % 2) == 1 {
                    frame.digits[pd].b = true;
                    frame.digits[pd].c = true;
                } else {
                    frame.digits[pd].f = true;
                    frame.digits[pd].e = true;
                }
            } else if !is_next && pos < 7 {
                let nd = (pos + 1) / 2;
                if ((pos + 1) % 2) == 1 {
                    frame.digits[nd].b = true;
                    frame.digits[nd].c = true;
                } else {
                    frame.digits[nd].f = true;
                    frame.digits[nd].e = true;
                }
            }
        }
        frame
    }

    /// 2. Бобины / Катушки магнитофона (Tape Reels: вращение 1 и 4 + натянутая лента)
    pub fn tape_track(step: usize, is_next: bool) -> Self {
        let mut frame = Self::default();
        // Лента по центру
        frame.digits[1].d = true;
        frame.digits[2].d = true;
        frame.minus = true;

        let phase = if is_next { step % 4 } else { (3 - (step % 4)) % 4 };
        match phase {
            0 => {
                // Фаза 1: вертикаль A + D
                frame.digits[0].a = true; frame.digits[0].d = true;
                frame.digits[3].a = true; frame.digits[3].d = true;
            }
            1 => {
                // Фаза 2: диагональ B + E
                frame.digits[0].b = true; frame.digits[0].e = true;
                frame.digits[3].b = true; frame.digits[3].e = true;
            }
            2 => {
                // Фаза 3: диагональ C + F
                frame.digits[0].c = true; frame.digits[0].f = true;
                frame.digits[3].c = true; frame.digits[3].f = true;
            }
            3 => {
                // Фаза 4: горизонталь G
                frame.digits[0].g = true;
                frame.digits[3].g = true;
            }
            _ => {}
        }
        frame
    }

    /// 3. Каскадный барабан (Slot Machine Roll: вертикальный перекат D -> G -> A)
    pub fn slot_track(step: usize, is_next: bool) -> Self {
        let mut frame = Self::default();
        if is_next {
            match step {
                0 => { frame.digits[0].d = true; }
                1 => { frame.digits[0].g = true; frame.digits[1].d = true; }
                2 => { frame.digits[0].a = true; frame.digits[1].g = true; frame.digits[2].d = true; }
                3 => { frame.digits[1].a = true; frame.digits[2].g = true; frame.digits[3].d = true; }
                4 => { frame.digits[2].a = true; frame.digits[3].g = true; }
                5 => { frame.digits[3].a = true; }
                6 | 7 => {
                    frame.digits[1] = char_to_segments('>');
                    frame.digits[2] = char_to_segments('>');
                }
                _ => {}
            }
        } else {
            match step {
                0 => { frame.digits[3].a = true; }
                1 => { frame.digits[3].g = true; frame.digits[2].a = true; }
                2 => { frame.digits[3].d = true; frame.digits[2].g = true; frame.digits[1].a = true; }
                3 => { frame.digits[2].d = true; frame.digits[1].g = true; frame.digits[0].a = true; }
                4 => { frame.digits[1].d = true; frame.digits[0].g = true; }
                5 => { frame.digits[0].d = true; }
                6 | 7 => {
                    frame.digits[1] = char_to_segments('<');
                    frame.digits[2] = char_to_segments('<');
                }
                _ => {}
            }
        }
        frame
    }

    /// 4. Бегущий шеврон (Chevron Slide: стрелка ускоряется и переходит в >>)
    pub fn chevron_track(step: usize, is_next: bool) -> Self {
        let mut frame = Self::default();
        let ch = if is_next { '>' } else { '<' };
        if is_next {
            match step {
                0 => { frame.digits[0] = char_to_segments(ch); }
                1 => { frame.digits[1] = char_to_segments(ch); }
                2 => { frame.digits[2] = char_to_segments(ch); }
                3 => { frame.digits[3] = char_to_segments(ch); }
                4 | 5 => {
                    frame.digits[1] = char_to_segments(ch);
                    frame.digits[2] = char_to_segments(ch);
                }
                _ => {}
            }
        } else {
            match step {
                0 => { frame.digits[3] = char_to_segments(ch); }
                1 => { frame.digits[2] = char_to_segments(ch); }
                2 => { frame.digits[1] = char_to_segments(ch); }
                3 => { frame.digits[0] = char_to_segments(ch); }
                4 | 5 => {
                    frame.digits[1] = char_to_segments(ch);
                    frame.digits[2] = char_to_segments(ch);
                }
                _ => {}
            }
        }
        frame
    }

    /// Базовая обратная совместимость
    pub fn next_track(step: usize) -> Self {
        Self::chevron_track(step, true)
    }

    pub fn prev_track(step: usize) -> Self {
        Self::chevron_track(step, false)
    }

    /// Часы HH:MM
    pub fn clock(hours: u32, minutes: u32, colon: bool) -> Self {
        let mut frame = Self::default();
        let text = format!("{:02}{:02}", hours % 24, minutes % 60);
        let chars: Vec<char> = text.chars().collect();
        for i in 0..4 {
            if i < chars.len() {
                frame.digits[i] = char_to_segments(chars[i]);
            }
        }
        frame.colon_upper = colon;
        frame.colon_lower = colon;
        frame
    }
}

/// Аппаратный драйвер вывода в sysfs светодиодов Станции Мини 2
pub struct LedHardwareDriver {
    segment_paths: Vec<(String, PathBuf)>,
    last_state: [bool; 32],
    is_available: bool,
}

impl LedHardwareDriver {
    pub fn new() -> Self {
        let mut paths = Vec::with_capacity(32);
        let segments = ['A', 'B', 'C', 'D', 'E', 'F', 'G'];

        // 4 разряда по 7 сегментов (28 светодиодов)
        for digit in 1..=4 {
            for &seg in &segments {
                let name = format!("clock{}{}", seg, digit);
                let path = PathBuf::from(format!("/sys/class/leds/mini2:white:{}/brightness", name));
                paths.push((name, path));
            }
        }

        // 4 дополнительных элемента (двоеточие, минус, градус)
        paths.push(("clockPLUS1".to_string(), PathBuf::from("/sys/class/leds/mini2:white:clockPLUS1/brightness")));
        paths.push(("clockPLUS2".to_string(), PathBuf::from("/sys/class/leds/mini2:white:clockPLUS2/brightness")));
        paths.push(("clockMINUS".to_string(), PathBuf::from("/sys/class/leds/mini2:white:clockMINUS/brightness")));
        paths.push(("clockDEGREE".to_string(), PathBuf::from("/sys/class/leds/mini2:white:clockDEGREE/brightness")));

        let is_available = paths.iter().any(|(_, p)| p.exists());
        if is_available {
            info!("✨ Аппаратный LED-экран Станции Мини 2 обнаружен (32 сегмента)");
        } else {
            debug!("Аппаратные sysfs пути LED-экрана не найдены (эмуляция или другая ревизия)");
        }

        Self {
            segment_paths: paths,
            last_state: [false; 32],
            is_available,
        }
    }

    /// Преобразует `DisplayFrame` в плоский массив из 32 булевых значений
    fn frame_to_bits(frame: &DisplayFrame) -> [bool; 32] {
        let mut bits = [false; 32];
        let mut idx = 0;

        for digit in &frame.digits {
            bits[idx] = digit.a; idx += 1;
            bits[idx] = digit.b; idx += 1;
            bits[idx] = digit.c; idx += 1;
            bits[idx] = digit.d; idx += 1;
            bits[idx] = digit.e; idx += 1;
            bits[idx] = digit.f; idx += 1;
            bits[idx] = digit.g; idx += 1;
        }

        bits[idx] = frame.colon_upper; idx += 1;
        bits[idx] = frame.colon_lower; idx += 1;
        bits[idx] = frame.minus; idx += 1;
        bits[idx] = frame.degree;

        bits
    }

    /// Быстрая отрисовка кадра: обновляет в sysfs ТОЛЬКО изменившиеся сегменты
    pub fn render(&mut self, frame: &DisplayFrame, brightness: u8) {
        if !self.is_available {
            return;
        }

        let current_bits = Self::frame_to_bits(frame);
        let on_str = format!("{}\n", brightness);
        let off_str = "0\n";

        for (i, &state) in current_bits.iter().enumerate() {
            if state != self.last_state[i] {
                let (_, path) = &self.segment_paths[i];
                let val_to_write = if state { on_str.as_bytes() } else { off_str.as_bytes() };
                let _ = std::fs::write(path, val_to_write);
                self.last_state[i] = state;
            }
        }
    }

    /// Принудительная безусловная перезапись всех 32 сегментов в sysfs (для сброса артефактов Алисы)
    pub fn force_render(&mut self, frame: &DisplayFrame, brightness: u8) {
        if !self.is_available {
            return;
        }

        let current_bits = Self::frame_to_bits(frame);
        let on_str = format!("{}\n", brightness);
        let off_str = "0\n";

        for (i, &state) in current_bits.iter().enumerate() {
            let (_, path) = &self.segment_paths[i];
            let val_to_write = if state { on_str.as_bytes() } else { off_str.as_bytes() };
            let _ = std::fs::write(path, val_to_write);
            self.last_state[i] = state;
        }
    }

    /// Сброс кэша состояния и гашение экрана: следующий кадр гарантированно запишет все 32 сегмента
    pub fn invalidate(&mut self) {
        if !self.is_available {
            return;
        }
        for (_, path) in &self.segment_paths {
            let _ = std::fs::write(path, b"0\n");
        }
        self.last_state = [false; 32];
    }

    /// Полное погасание всех сегментов
    pub fn turn_off_all(&mut self) {
        if !self.is_available {
            return;
        }
        for (i, (_, path)) in self.segment_paths.iter().enumerate() {
            let _ = std::fs::write(path, b"0\n");
            self.last_state[i] = false;
        }
    }
}
