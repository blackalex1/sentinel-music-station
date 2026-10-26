use std::path::PathBuf;
use tracing::{debug, info};

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
        _ => DigitSegments::default(),
    }
}

impl DisplayFrame {
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

    /// Кадр анимации переключения на следующий трек (Next >>)
    pub fn next_track(step: usize) -> Self {
        let mut frame = Self::default();
        match step {
            0 => {
                frame.digits[0] = char_to_segments('>');
            }
            1 => {
                frame.digits[0] = char_to_segments('>');
                frame.digits[1] = char_to_segments('>');
            }
            2 => {
                frame.digits[1] = char_to_segments('>');
                frame.digits[2] = char_to_segments('>');
            }
            3 => {
                frame.digits[2] = char_to_segments('>');
                frame.digits[3] = char_to_segments('>');
            }
            4 => {
                frame.digits[3] = char_to_segments('>');
            }
            _ => {}
        }
        frame
    }

    /// Кадр анимации переключения на предыдущий трек (Prev <<)
    pub fn prev_track(step: usize) -> Self {
        let mut frame = Self::default();
        match step {
            0 => {
                frame.digits[3] = char_to_segments('<');
            }
            1 => {
                frame.digits[3] = char_to_segments('<');
                frame.digits[2] = char_to_segments('<');
            }
            2 => {
                frame.digits[2] = char_to_segments('<');
                frame.digits[1] = char_to_segments('<');
            }
            3 => {
                frame.digits[1] = char_to_segments('<');
                frame.digits[0] = char_to_segments('<');
            }
            4 => {
                frame.digits[0] = char_to_segments('<');
            }
            _ => {}
        }
        frame
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
