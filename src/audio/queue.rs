use std::time::{SystemTime, UNIX_EPOCH};
use crate::connect::protocol::TrackMeta;

/// Простой легковесный генератор псевдослучайных чисел Xorshift64 (без внешних зависимостей)
#[derive(Debug)]
struct SimpleRng(u64);

impl SimpleRng {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(987654321);
        let mut seed = nanos ^ 0x517cc1b727220a95;
        if seed == 0 {
            seed = 123456789;
        }
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_usize(&mut self, max: usize) -> usize {
        if max == 0 {
            0
        } else {
            (self.next_u64() % (max as u64)) as usize
        }
    }
}

/// Менеджер очереди воспроизведения треков с поддержкой режима Shuffle
#[derive(Debug)]
pub struct PlaybackQueue {
    tracks: Vec<TrackMeta>,
    order: Vec<usize>,
    current_index: usize,
    is_shuffle: bool,
    rng: SimpleRng,
}

impl Default for PlaybackQueue {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            order: Vec::new(),
            current_index: 0,
            is_shuffle: false,
            rng: SimpleRng::new(),
        }
    }
}

impl PlaybackQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Установка новой очереди и выбор начального трека
    pub fn set_queue(&mut self, tracks: Vec<TrackMeta>, start_track_id: i64) {
        if tracks.is_empty() {
            self.tracks.clear();
            self.order.clear();
            self.current_index = 0;
            return;
        }

        let start_pos = tracks.iter().position(|t| t.id == start_track_id).unwrap_or(0);
        let len = tracks.len();
        self.tracks = tracks;

        if self.is_shuffle {
            self.build_shuffled_order(start_pos);
            self.current_index = 0;
        } else {
            self.order = (0..len).collect();
            self.current_index = start_pos;
        }
    }

    /// Включение или отключение режима перемешивания (Shuffle)
    pub fn set_shuffle(&mut self, shuffle: bool) {
        if self.is_shuffle == shuffle {
            return;
        }

        self.is_shuffle = shuffle;
        if self.tracks.is_empty() {
            return;
        }

        let current_real_idx = self.order.get(self.current_index).copied().unwrap_or(0);

        if self.is_shuffle {
            // Перемешиваем очередь, сохраняя текущий трек на первой позиции
            self.build_shuffled_order(current_real_idx);
            self.current_index = 0;
        } else {
            // Восстанавливаем естественный порядок треклиста
            self.order = (0..self.tracks.len()).collect();
            self.current_index = current_real_idx;
        }
    }

    /// Переключение режима перемешивания
    pub fn toggle_shuffle(&mut self) -> bool {
        self.set_shuffle(!self.is_shuffle);
        self.is_shuffle
    }

    /// Проверка статуса перемешивания
    pub fn is_shuffle(&self) -> bool {
        self.is_shuffle
    }

    /// Создание случайного порядка воспроизведения (алгоритм Фишера-Йетса)
    fn build_shuffled_order(&mut self, start_real_idx: usize) {
        let len = self.tracks.len();
        let mut remaining: Vec<usize> = (0..len).filter(|&idx| idx != start_real_idx).collect();

        // Перемешивание остальных треков
        if remaining.len() > 1 {
            for i in (1..remaining.len()).rev() {
                let j = self.rng.next_usize(i + 1);
                remaining.swap(i, j);
            }
        }

        let mut new_order = Vec::with_capacity(len);
        new_order.push(start_real_idx);
        new_order.extend(remaining);
        self.order = new_order;
    }

    /// Проверка, пуста ли очередь
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Текущий активный трек
    pub fn current(&self) -> Option<&TrackMeta> {
        let real_idx = *self.order.get(self.current_index)?;
        self.tracks.get(real_idx)
    }

    /// Переход к следующему треку (с циклическим возвратом в начало)
    pub fn next(&mut self) -> Option<TrackMeta> {
        if self.tracks.is_empty() {
            return None;
        }
        if self.order.len() != self.tracks.len() {
            self.order = (0..self.tracks.len()).collect();
        }

        self.current_index = (self.current_index + 1) % self.order.len();
        let real_idx = self.order[self.current_index];
        self.tracks.get(real_idx).cloned()
    }

    /// Переход к предыдущему треку (с циклическим возвратом в конец)
    pub fn prev(&mut self) -> Option<TrackMeta> {
        if self.tracks.is_empty() {
            return None;
        }
        if self.order.len() != self.tracks.len() {
            self.order = (0..self.tracks.len()).collect();
        }

        if self.current_index > 0 {
            self.current_index -= 1;
        } else {
            self.current_index = self.order.len().saturating_sub(1);
        }
        let real_idx = self.order[self.current_index];
        self.tracks.get(real_idx).cloned()
    }

    /// Получение копии текущего списка треков
    pub fn tracks(&self) -> Vec<TrackMeta> {
        self.tracks.clone()
    }
}
