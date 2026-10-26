use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

const BLOCKS: [char; 8] = [' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Отрисовка полос спектрограммы с использованием символов Unicode ( ▂▃▄▅▆▇█) в виде текста
pub fn render_ascii_bars(bands: &[f32], max_height: usize) -> String {
    if max_height <= 1 {
        return bands
            .iter()
            .map(|&val| {
                let idx = ((val * (BLOCKS.len() - 1) as f32).round() as usize).min(BLOCKS.len() - 1);
                BLOCKS[idx]
            })
            .collect();
    }

    let mut lines = Vec::with_capacity(max_height);
    for row in (0..max_height).rev() {
        let mut line = String::with_capacity(bands.len() * 2);
        let row_threshold = row as f32 / max_height as f32;
        let next_threshold = (row + 1) as f32 / max_height as f32;

        for &val in bands {
            if val >= next_threshold {
                line.push('█');
                line.push(' ');
            } else if val > row_threshold {
                let frac = (val - row_threshold) / (next_threshold - row_threshold);
                let idx = ((frac * (BLOCKS.len() - 1) as f32).round() as usize).min(BLOCKS.len() - 1);
                line.push(BLOCKS[idx]);
                line.push(' ');
            } else {
                line.push(' ');
                line.push(' ');
            }
        }
        lines.push(line);
    }

    lines.join("\n")
}

/// Полноэкранный градиентный рендеринг спектрограммы для Ratatui
pub fn render_spectrum_lines(bands: &[f32], max_height: usize, max_width: usize) -> Vec<Line<'static>> {
    if max_height < 3 || bands.is_empty() {
        return Vec::new();
    }

    // Резервируем нижние 2 строки для частотной шкалы
    let eff_height = (max_height - 2).max(1);
    let num_bands = bands.len();

    // Вычисляем ширину одной полосы в символах
    let spacing = 1;
    let total_spacing = (num_bands - 1) * spacing;
    let bar_width = if max_width > total_spacing {
        ((max_width - total_spacing) / num_bands).clamp(1, 8)
    } else {
        1
    };

    let total_used_width = num_bands * bar_width + total_spacing;
    let left_pad = (max_width.saturating_sub(total_used_width)) / 2;
    let pad_str = " ".repeat(left_pad);

    let mut lines = Vec::with_capacity(max_height);

    for row in (0..eff_height).rev() {
        let h_ratio = (row + 1) as f32 / eff_height as f32;
        let row_color = if h_ratio > 0.82 {
            Color::LightRed
        } else if h_ratio > 0.58 {
            Color::Yellow
        } else if h_ratio > 0.32 {
            Color::Green
        } else {
            Color::Cyan
        };

        let row_threshold = row as f32 / eff_height as f32;
        let next_threshold = (row + 1) as f32 / eff_height as f32;

        let mut spans = Vec::new();
        if left_pad > 0 {
            spans.push(Span::raw(pad_str.clone()));
        }

        for (i, &val) in bands.iter().enumerate() {
            let ch = if val >= next_threshold {
                '█'
            } else if val > row_threshold {
                let frac = (val - row_threshold) / (next_threshold - row_threshold);
                let idx = ((frac * (BLOCKS.len() - 1) as f32).round() as usize).min(BLOCKS.len() - 1);
                BLOCKS[idx]
            } else if row == 0 {
                '·' // Базовый уровень покоя
            } else {
                ' '
            };

            let bar_segment = ch.to_string().repeat(bar_width);
            let style = if ch == '·' {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(row_color).add_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(bar_segment, style));

            if i + 1 < num_bands {
                spans.push(Span::raw(" ".repeat(spacing)));
            }
        }

        lines.push(Line::from(spans));
    }

    // Разделительная линия
    let mut div_spans = Vec::new();
    if left_pad > 0 {
        div_spans.push(Span::raw(pad_str.clone()));
    }
    div_spans.push(Span::styled(
        "─".repeat(total_used_width.min(max_width)),
        Style::default().fg(Color::DarkGray),
    ));
    lines.push(Line::from(div_spans));

    // Подписи частот
    let freq_labels = [
        "40Hz", "80Hz", "160Hz", "320Hz", "640Hz", "1.2kHz", "2.5kHz", "5kHz", "10kHz", "16kHz",
    ];
    let mut axis_text = String::with_capacity(total_used_width);
    let step = (total_used_width / freq_labels.len()).max(6);
    for label in &freq_labels {
        axis_text.push_str(label);
        let rem = step.saturating_sub(label.len());
        axis_text.push_str(&" ".repeat(rem));
    }

    let mut axis_spans = Vec::new();
    if left_pad > 0 {
        axis_spans.push(Span::raw(pad_str));
    }
    axis_spans.push(Span::styled(
        axis_text,
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM),
    ));
    lines.push(Line::from(axis_spans));

    lines
}

/// Экран ожидания (Standby HUD), когда трек не воспроизводится
pub fn render_standby_lines(max_height: usize, _max_width: usize, device_name: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    let banner_top = "╔══════════════════════════════════════════════════════════════════╗";
    let banner_title = "║                   SENTINEL AUDIO ECOSYSTEM                   ║";
    let banner_sub = "║                   РЕЖИМ ОЖИДАНИЯ • STANDBY                   ║";
    let banner_bot = "╚══════════════════════════════════════════════════════════════════╝";

    let pad_top = (max_height.saturating_sub(15)) / 2;
    for _ in 0..pad_top {
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(banner_top, Style::default().fg(Color::Cyan))));
    lines.push(Line::from(Span::styled(banner_title, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))));
    lines.push(Line::from(Span::styled(banner_sub, Style::default().fg(Color::Yellow))));
    lines.push(Line::from(Span::styled(banner_bot, Style::default().fg(Color::Cyan))));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![
        Span::styled("     [Пробел]   ", Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("  Начать / возобновить воспроизведение"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    [N] / [→]   ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw("  Следующий трек очереди"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    [P] / [←]   ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw("  Предыдущий трек очереди"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("      [+/-]     ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw("  Громкость звука ALSA"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("       [M]      ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw("  Быстрое отключение звука (Mute)"),
    ]));
    lines.push(Line::from(vec![
        Span::styled("    [Q / Esc]   ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw("  Выход из TUI в командную строку"),
    ]));

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  📡 Устройство: ", Style::default().fg(Color::DarkGray)),
        Span::styled(device_name.to_string(), Style::default().fg(Color::LightGreen).add_modifier(Modifier::BOLD)),
        Span::styled(" • ALSA Native Driver", Style::default().fg(Color::DarkGray)),
    ]));

    lines
}
