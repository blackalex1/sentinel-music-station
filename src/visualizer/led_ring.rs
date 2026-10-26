use std::fs;
use std::path::PathBuf;
use tracing::{debug, info};

/// Официальная таблица цветов индикации громкости Станции Мини 2 (из oknix/volume.led).
/// 21 градация от 0% (темно-зеленый) через лаймовый и оранжевый к 100% (ярко-красный).
pub const VOLUME_COLORS: [(u8, u8, u8); 21] = [
    (0x11, 0x61, 0x11), // 0%   (116111) Темно-зеленый
    (0x1D, 0x76, 0x1D), // 5%   (1D761D)
    (0x2D, 0x86, 0x24), // 10%  (2D8624)
    (0x3A, 0x95, 0x26), // 15%  (3A9526)
    (0x4A, 0xB3, 0x24), // 20%  (4AB324)
    (0x56, 0xC0, 0x1D), // 25%  (56C01D) Зеленый
    (0x65, 0xCC, 0x15), // 30%  (65CC15)
    (0x77, 0xD9, 0x0B), // 35%  (77D90B)
    (0x8D, 0xE5, 0x00), // 40%  (8DE500) Лайм
    (0xA9, 0xF1, 0x00), // 45%  (A9F100)
    (0xC9, 0xFA, 0x00), // 50%  (C9FA00) Желто-зеленый
    (0xE9, 0xEC, 0x00), // 55%  (E9EC00) Желтый
    (0xEC, 0xD4, 0x00), // 60%  (ECD400) Янтарно-желтый
    (0xEC, 0xC5, 0x00), // 65%  (ECC500)
    (0xEC, 0xB2, 0x00), // 70%  (ECB200) Янтарно-оранжевый
    (0xEB, 0x9E, 0x00), // 75%  (EB9E00)
    (0xEB, 0x8B, 0x00), // 80%  (EB8B00) Оранжевый
    (0xEC, 0x59, 0x00), // 85%  (EC5900) Темно-оранжевый
    (0xED, 0x4C, 0x00), // 90%  (ED4C00) Красно-оранжевый
    (0xF0, 0x32, 0x00), // 95%  (F03200) Ярко-красный
    (0xF8, 0x00, 0x00), // 100% (F80000) Насыщенный красный
];

/// Вычисляет цвет (R, G, B) для заданного уровня громкости (0.0..1.0)
pub fn volume_to_color(volume: f32) -> (u8, u8, u8) {
    let step = (volume.clamp(0.0, 1.0) * 20.0).round() as usize;
    VOLUME_COLORS[step.min(20)]
}

struct LedChannelPaths {
    r: PathBuf,
    g: PathBuf,
    b: PathBuf,
}

/// Аппаратный драйвер круглого кольца RGB-подсветки кнопки Станции Мини 2 (чип IS31FL3236 на i2c-1).
/// 6 полноцветных RGB светодиодов под центральной сенсорной кнопкой.
pub struct LedRingDriver {
    channels: [LedChannelPaths; 6],
    last_colors: [(u8, u8, u8); 6],
    is_available: bool,
}

impl LedRingDriver {
    pub fn new() -> Self {
        let channels = [
            LedChannelPaths {
                r: PathBuf::from("/sys/class/leds/mini2:red:ring1/brightness"),
                g: PathBuf::from("/sys/class/leds/mini2:green:ring1/brightness"),
                b: PathBuf::from("/sys/class/leds/mini2:blue:ring1/brightness"),
            },
            LedChannelPaths {
                r: PathBuf::from("/sys/class/leds/mini2:red:ring2/brightness"),
                g: PathBuf::from("/sys/class/leds/mini2:green:ring2/brightness"),
                b: PathBuf::from("/sys/class/leds/mini2:blue:ring2/brightness"),
            },
            LedChannelPaths {
                r: PathBuf::from("/sys/class/leds/mini2:red:ring3/brightness"),
                g: PathBuf::from("/sys/class/leds/mini2:green:ring3/brightness"),
                b: PathBuf::from("/sys/class/leds/mini2:blue:ring3/brightness"),
            },
            LedChannelPaths {
                r: PathBuf::from("/sys/class/leds/mini2:red:ring4/brightness"),
                g: PathBuf::from("/sys/class/leds/mini2:green:ring4/brightness"),
                b: PathBuf::from("/sys/class/leds/mini2:blue:ring4/brightness"),
            },
            LedChannelPaths {
                r: PathBuf::from("/sys/class/leds/mini2:red:ring5/brightness"),
                g: PathBuf::from("/sys/class/leds/mini2:green:ring5/brightness"),
                b: PathBuf::from("/sys/class/leds/mini2:blue:ring5/brightness"),
            },
            LedChannelPaths {
                r: PathBuf::from("/sys/class/leds/mini2:red:ring6/brightness"),
                g: PathBuf::from("/sys/class/leds/mini2:green:ring6/brightness"),
                b: PathBuf::from("/sys/class/leds/mini2:blue:ring6/brightness"),
            },
        ];

        let is_available = channels[0].r.exists();
        if is_available {
            info!("✨ Аппаратное кольцо подсветки кнопки Станции Мини 2 обнаружено (6 RGB светодиодов)");
        } else {
            debug!("Аппаратные sysfs пути кольца подсветки не найдены");
        }

        Self {
            channels,
            last_colors: [(0, 0, 0); 6],
            is_available,
        }
    }

    /// Установка индивидуальных цветов для каждого из 6 светодиодов (обновляет в sysfs только изменившиеся каналы)
    pub fn set_colors(&mut self, colors: &[(u8, u8, u8); 6]) {
        if !self.is_available {
            return;
        }

        for (i, &(r, g, b)) in colors.iter().enumerate() {
            let (lr, lg, lb) = self.last_colors[i];
            if r != lr {
                let _ = fs::write(&self.channels[i].r, format!("{}\n", r));
            }
            if g != lg {
                let _ = fs::write(&self.channels[i].g, format!("{}\n", g));
            }
            if b != lb {
                let _ = fs::write(&self.channels[i].b, format!("{}\n", b));
            }
            self.last_colors[i] = (r, g, b);
        }
    }

    /// Установка единого сплошного цвета для всех 6 светодиодов кольца
    pub fn set_solid(&mut self, r: u8, g: u8, b: u8) {
        self.set_colors(&[(r, g, b); 6]);
    }

    /// Полное выключение подсветки кольца
    pub fn turn_off(&mut self) {
        self.set_solid(0, 0, 0);
    }
}
