use std::sync::{Arc, Mutex};
use std::time::Duration;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Gauge, Paragraph},
    Frame, Terminal,
};
use tokio::sync::{mpsc, watch};

use crate::config::StationConfig;
use crate::connect::protocol::{PlayerCommand, PlayerState};
use crate::visualizer::{render_ascii_bars, SpectrumAnalyzer};

pub struct TuiApp {
    config: Arc<StationConfig>,
    command_tx: mpsc::Sender<PlayerCommand>,
    state_rx: watch::Receiver<PlayerState>,
    is_connected_rx: watch::Receiver<bool>,
    sample_buffer: Arc<Mutex<Vec<f32>>>,
    spectrum_analyzer: SpectrumAnalyzer,
}

impl TuiApp {
    pub fn new(
        config: Arc<StationConfig>,
        command_tx: mpsc::Sender<PlayerCommand>,
        state_rx: watch::Receiver<PlayerState>,
        is_connected_rx: watch::Receiver<bool>,
        sample_buffer: Arc<Mutex<Vec<f32>>>,
    ) -> Self {
        Self {
            config,
            command_tx,
            state_rx,
            is_connected_rx,
            sample_buffer,
            spectrum_analyzer: SpectrumAnalyzer::new(1024, 16),
        }
    }

    /// Запуск цикла отрисовки TUI и обработки клавиш
    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        let mut last_draw = std::time::Instant::now();
        let frame_duration = Duration::from_millis(33); // ~30 FPS

        loop {
            let samples = if let Ok(buf) = self.sample_buffer.lock() {
                buf.clone()
            } else {
                Vec::new()
            };

            let bands = if !samples.is_empty() {
                self.spectrum_analyzer.compute(&samples)
            } else {
                let state = self.state_rx.borrow();
                if state.is_playing {
                    Self::generate_live_bands(state.position_ms)
                } else {
                    vec![0.0; 16]
                }
            };

            terminal.draw(|f| {
                self.draw(f, &bands);
            })?;

            // Обработка ввода с клавиатуры с таймаутом
            let elapsed = last_draw.elapsed();
            let timeout = frame_duration.saturating_sub(elapsed);
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            break;
                        }
                        KeyCode::Char(' ') => {
                            let _ = self.command_tx.send(PlayerCommand::TogglePlay).await;
                        }
                        KeyCode::Char('n') | KeyCode::Right => {
                            let _ = self.command_tx.send(PlayerCommand::Next).await;
                        }
                        KeyCode::Char('p') | KeyCode::Left => {
                            let _ = self.command_tx.send(PlayerCommand::Prev).await;
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Up => {
                            let current_vol = self.state_rx.borrow().volume;
                            let new_vol = (current_vol + 0.05).min(1.0);
                            let _ = self.command_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
                        }
                        KeyCode::Char('-') | KeyCode::Down => {
                            let current_vol = self.state_rx.borrow().volume;
                            let new_vol = (current_vol - 0.05).max(0.0);
                            let _ = self.command_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
                        }
                        KeyCode::Char('m') => {
                            let current_vol = self.state_rx.borrow().volume;
                            let new_vol = if current_vol > 0.0 { 0.0 } else { 0.7 };
                            let _ = self.command_tx.send(PlayerCommand::SetVolume { volume: new_vol }).await;
                        }
                        _ => {}
                    }
                }
            }

            last_draw = std::time::Instant::now();
        }

        Ok(())
    }

    fn draw(&self, f: &mut Frame, bands: &[f32]) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),  // Шапка (Статус, имя)
                Constraint::Length(7),  // Текущий трек и прогресс
                Constraint::Min(6),     // Анализатор спектра (FFT)
                Constraint::Length(3),  // Панель подсказок / клавиш
            ])
            .split(f.area());

        self.draw_header(f, chunks[0]);
        self.draw_player(f, chunks[1]);
        self.draw_visualizer(f, chunks[2], bands);
        self.draw_footer(f, chunks[3]);
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let is_connected = *self.is_connected_rx.borrow();
        let (status_text, status_color) = if is_connected {
            ("🟢 В СЕТИ (Connected)", Color::Green)
        } else {
            ("🔴 ОФЛАЙН (Reconnecting...)", Color::Red)
        };

        let header_text = vec![Line::from(vec![
            Span::styled(" 📡 SENTINEL STATION ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" | Колонка: "),
            Span::styled(&self.config.device_name, Style::default().fg(Color::Yellow)),
            Span::raw(" | Статус: "),
            Span::styled(status_text, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            Span::raw(" | Сервер: "),
            Span::styled(&self.config.server_url, Style::default().fg(Color::DarkGray)),
        ])];

        let p = Paragraph::new(header_text)
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));
        f.render_widget(p, area);
    }

    fn draw_player(&self, f: &mut Frame, area: Rect) {
        let state = self.state_rx.borrow().clone();

        let track_title = state.current_track.as_ref().map_or("Музыка не играет", |t| &t.title);
        let track_artist = state.current_track.as_ref().map_or("—", |t| &t.artist);

        let status_icon = if state.is_playing { "▶ Играет" } else { "⏸ На паузе" };
        let volume_percent = (state.volume * 100.0).round() as u32;

        let pos_secs = state.position_ms / 1000;
        let dur_secs = state.duration_ms / 1000;
        let pos_str = format!("{:02}:{:02}", pos_secs / 60, pos_secs % 60);
        let dur_str = format!("{:02}:{:02}", dur_secs / 60, dur_secs % 60);

        let progress_ratio = if state.duration_ms > 0 {
            (state.position_ms as f64 / state.duration_ms as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let player_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Название и артист
                Constraint::Length(2), // Прогресс-бар
                Constraint::Length(1), // Громкость и тайминги
            ])
            .split(Block::default()
                .title(" 🎵 СЕЙЧАС ИГРАЕТ ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .inner(area));

        f.render_widget(Block::default().title(" 🎵 СЕЙЧАС ИГРАЕТ ").borders(Borders::ALL).border_type(BorderType::Rounded), area);

        let track_info = Paragraph::new(vec![
            Line::from(vec![
                Span::styled(format!("  {}  ", status_icon), Style::default().fg(if state.is_playing { Color::Green } else { Color::Yellow })),
                Span::styled(track_title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw(" — "),
                Span::styled(track_artist, Style::default().fg(Color::LightCyan)),
            ])
        ]);
        f.render_widget(track_info, player_chunks[0]);

        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
            .ratio(progress_ratio)
            .label(format!("{} / {}", pos_str, dur_str));
        f.render_widget(gauge, player_chunks[1]);

        let meta_line = Paragraph::new(vec![
            Line::from(vec![
                Span::raw(format!("  🔊 Громкость: {}%  |  ALSA: {}", volume_percent, self.config.audio_device)),
            ])
        ]);
        f.render_widget(meta_line, player_chunks[2]);
    }

    fn draw_visualizer(&self, f: &mut Frame, area: Rect, bands: &[f32]) {
        let block = Block::default()
            .title(" 📊 СПЕКТРОГРАММА В РЕАЛЬНОМ ВРЕМЕНИ (FFT) ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded);

        let inner = block.inner(area);
        f.render_widget(block, area);

        if inner.height < 2 {
            return;
        }

        let ascii_bars = render_ascii_bars(bands, inner.height as usize);
        let paragraph = Paragraph::new(ascii_bars)
            .style(Style::default().fg(Color::LightGreen))
            .alignment(Alignment::Center);

        f.render_widget(paragraph, inner);
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let help = Paragraph::new(Line::from(vec![
            Span::styled(" [Пробел] ", Style::default().fg(Color::Black).bg(Color::White)),
            Span::raw(" Пауза/Старт  "),
            Span::styled(" [N/→] ", Style::default().fg(Color::Black).bg(Color::White)),
            Span::raw(" След  "),
            Span::styled(" [P/←] ", Style::default().fg(Color::Black).bg(Color::White)),
            Span::raw(" Пред  "),
            Span::styled(" [+/-] ", Style::default().fg(Color::Black).bg(Color::White)),
            Span::raw(" Громкость  "),
            Span::styled(" [M] ", Style::default().fg(Color::Black).bg(Color::White)),
            Span::raw(" Без звука  "),
            Span::styled(" [Q/Esc] ", Style::default().fg(Color::Black).bg(Color::White)),
            Span::raw(" Выход в консоль "),
        ]))
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded));

        f.render_widget(help, area);
    }

    /// Синтез живых полос спектрограммы для удаленного TUI-терминала
    fn generate_live_bands(position_ms: u64) -> Vec<f32> {
        let t = position_ms as f32 / 1000.0;
        let mut bands = Vec::with_capacity(16);
        for i in 0..16 {
            let fi = (i + 1) as f32;
            let wave1 = (t * 3.5 * (1.0 + fi * 0.08) + fi * 0.7).sin();
            let wave2 = (t * 7.0 - fi * 0.35).cos();
            let wave3 = (t * 1.8 + fi * 1.1).sin();

            let bass_boost = if i < 4 { 0.25 } else if i < 8 { 0.1 } else { 0.0 };
            let val = ((wave1 * 0.4 + wave2 * 0.35 + wave3 * 0.25).abs() + bass_boost)
                .clamp(0.04, 0.96);
            bands.push(val);
        }
        bands
    }
}
