use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use rodio::Source;

/// Потоковый аудиоисточник с фоновым декодированием в отдельном потоке
/// Буферизует ~1-2 секунды PCM-сэмплов в канале между потоком декодирования и ALSA
pub struct BufferedStreamSource {
    receiver: mpsc::Receiver<Vec<f32>>,
    current_chunk: Option<Vec<f32>>,
    chunk_index: usize,
    channels: u16,
    sample_rate: u32,
    total_duration: Option<Duration>,
    stop_flag: Arc<AtomicBool>,
}

impl BufferedStreamSource {
    pub fn new<I>(
        source: I,
        sample_buffer: Arc<Mutex<Vec<f32>>>,
        visualizer_enabled: bool,
    ) -> Self
    where
        I: Source<Item = f32> + Send + 'static,
    {
        let channels = source.channels();
        let sample_rate = source.sample_rate();
        let total_duration = source.total_duration();
        let stop_flag = Arc::new(AtomicBool::new(false));
        let thread_stop = stop_flag.clone();

        // Буфер на 16 блоков по 2048 сэмплов (~0.7-1.5 сек предзагрузки)
        let (tx, rx) = mpsc::sync_channel::<Vec<f32>>(16);

        std::thread::Builder::new()
            .name("sentinel-decoder".to_string())
            .spawn(move || {
                let mut source = source;
                let chunk_size = 2048;
                let mut chunk = Vec::with_capacity(chunk_size);

                loop {
                    if thread_stop.load(Ordering::Relaxed) {
                        break;
                    }

                    match source.next() {
                        Some(sample) => {
                            chunk.push(sample);
                            if chunk.len() >= chunk_size {
                                if visualizer_enabled {
                                    if let Ok(mut buf) = sample_buffer.try_lock() {
                                        buf.clear();
                                        buf.extend_from_slice(&chunk);
                                    }
                                }

                                if tx.send(chunk).is_err() || thread_stop.load(Ordering::Relaxed) {
                                    // Плеер закрылся, переключил трек или запросил остановку
                                    break;
                                }
                                chunk = Vec::with_capacity(chunk_size);
                            }
                        }
                        None => {
                            if !chunk.is_empty() && !thread_stop.load(Ordering::Relaxed) {
                                let _ = tx.send(chunk);
                            }
                            break;
                        }
                    }
                }
            })
            .expect("Не удалось запустить поток декодирования");

        Self {
            receiver: rx,
            current_chunk: None,
            chunk_index: 0,
            channels,
            sample_rate,
            total_duration,
            stop_flag,
        }
    }
}

impl Drop for BufferedStreamSource {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}

impl Iterator for BufferedStreamSource {
    type Item = f32;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref chunk) = self.current_chunk {
                if self.chunk_index < chunk.len() {
                    let sample = chunk[self.chunk_index];
                    self.chunk_index += 1;
                    return Some(sample);
                }
            }

            match self.receiver.recv() {
                Ok(chunk) => {
                    self.current_chunk = Some(chunk);
                    self.chunk_index = 0;
                }
                Err(_) => {
                    self.current_chunk = None;
                    return None;
                }
            }
        }
    }
}

impl Source for BufferedStreamSource {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.channels
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
    }
}
