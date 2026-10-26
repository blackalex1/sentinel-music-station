use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// Анализатор спектра на основе быстрого преобразования Фурье (FFT)
pub struct SpectrumAnalyzer {
    fft_size: usize,
    num_bands: usize,
    planner: FftPlanner<f32>,
    window: Vec<f32>,
    smoothed_bands: Vec<f32>,
    decay_rate: f32,
    attack_rate: f32,
}

impl SpectrumAnalyzer {
    pub fn new(fft_size: usize, num_bands: usize) -> Self {
        // Окно Ханна (Hann window) для сглаживания краев выборки сэмплов
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (fft_size - 1) as f32).cos())
            })
            .collect();

        Self {
            fft_size,
            num_bands,
            planner: FftPlanner::new(),
            window,
            smoothed_bands: vec![0.0; num_bands],
            decay_rate: 0.20,
            attack_rate: 0.65,
        }
    }

    /// Вычисление уровней частотных полос по PCM-сэмплам (возвращает нормализованные значения от 0.0 до 1.0)
    pub fn compute(&mut self, samples: &[f32]) -> Vec<f32> {
        if samples.len() < self.fft_size {
            // Если сэмплов недостаточно, плавно затухаем
            for b in self.smoothed_bands.iter_mut() {
                *b = (*b - self.decay_rate).max(0.0);
            }
            return self.smoothed_bands.clone();
        }

        // Берем последние fft_size сэмплов
        let start = samples.len() - self.fft_size;
        let mut buffer: Vec<Complex<f32>> = samples[start..]
            .iter()
            .zip(&self.window)
            .map(|(&s, &w)| Complex::new(s * w, 0.0))
            .collect();

        let fft = self.planner.plan_fft_forward(self.fft_size);
        fft.process(&mut buffer);

        // Получаем амплитуды спектра с правильной нормализацией на размер FFT
        let half = self.fft_size / 2;
        let norm_factor = 2.0 / (self.fft_size as f32);
        let magnitudes: Vec<f32> = buffer[..half]
            .iter()
            .map(|c| c.norm() * norm_factor)
            .collect();

        // 4 частотные полосы:
        // Полоса 0: Бас и суб-бас (~30 - 250 Гц)
        // Полоса 1: Нижняя середина (~250 - 1000 Гц)
        // Полоса 2: Середина и вокал (~1000 - 4000 Гц)
        // Полоса 3: Верха и тарелки (~4000 - 16000 Гц)
        let mut current_bands = vec![0.0; self.num_bands];
        let band_ranges = [
            (1, (half as f32 * 0.05) as usize),
            ((half as f32 * 0.05) as usize, (half as f32 * 0.18) as usize),
            ((half as f32 * 0.18) as usize, (half as f32 * 0.45) as usize),
            ((half as f32 * 0.45) as usize, half),
        ];

        // Психоакустическая компенсация слуха (высокие частоты в музыке имеют меньшую физическую амплитуду)
        let freq_boost = [1.0f32, 1.4, 2.0, 3.2];

        for (i, &(start_idx, end_idx)) in band_ranges.iter().enumerate() {
            let start_clamp = start_idx.min(half - 1);
            let end_clamp = end_idx.clamp(start_clamp + 1, half);
            let slice = &magnitudes[start_clamp..end_clamp];

            // Берем пиковое значение диапазона + долю среднего для сочности
            let peak = slice.iter().fold(0.0f32, |acc, &x| acc.max(x));
            let avg: f32 = slice.iter().sum::<f32>() / slice.len() as f32;
            let val = (peak * 0.8 + avg * 0.2) * freq_boost[i];

            // Динамический диапазон в децибелах (от -54 dBFS до -12 dBFS)
            let db = 20.0 * (val + 1e-6).log10();
            let normalized = ((db + 54.0) / 42.0).clamp(0.0, 1.0);
            current_bands[i] = normalized;
        }

        // Плавная анимация сглаживания (Attack/Decay)
        for (smoothed, &current) in self.smoothed_bands.iter_mut().zip(&current_bands) {
            if current > *smoothed {
                *smoothed += (current - *smoothed) * self.attack_rate;
            } else {
                *smoothed += (current - *smoothed) * self.decay_rate;
            }
            *smoothed = smoothed.clamp(0.0, 1.0);
        }

        self.smoothed_bands.clone()
    }
}
