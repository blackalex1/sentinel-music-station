use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use tracing::debug;

/// Драйвер вывода спектрограммы на аппаратную LED-матрицу Яндекс Станции Мини 2
pub struct LedMatrixController {
    device_path: Option<PathBuf>,
    width: usize,
    height: usize,
}

impl LedMatrixController {
    pub fn new(device_path: Option<String>, width: usize, height: usize) -> Self {
        Self {
            device_path: device_path.map(PathBuf::from),
            width,
            height,
        }
    }

    /// Отрисовка кадра спектра на аппаратный дисплей
    pub fn render_frame(&self, bands: &[f32]) {
        let path = match &self.device_path {
            Some(p) if p.exists() => p,
            _ => return,
        };

        // Формирование битовой маски или байтового буфера для колонок матрицы
        let mut frame_bytes = Vec::with_capacity(self.width);
        for x in 0..self.width {
            let band_val = if x < bands.len() { bands[x] } else { 0.0 };
            let bar_height = (band_val * self.height as f32).round() as usize;
            let bar_height = bar_height.min(self.height);

            // Битовая маска: 1 - светодиод горит, 0 - выключен
            let mut col_mask: u8 = 0;
            for y in 0..bar_height {
                col_mask |= 1 << y;
            }
            frame_bytes.push(col_mask);
        }

        // Запись кадра в устройство (например, /dev/spidev или /dev/matrix)
        if let Ok(mut file) = OpenOptions::new().write(true).open(path) {
            if let Err(e) = file.write_all(&frame_bytes) {
                debug!("Ошибка записи кадра в LED матрицу {}: {}", path.display(), e);
            }
        }
    }
}
