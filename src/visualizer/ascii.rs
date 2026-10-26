/// Отрисовка полос спектрограммы с использованием символов Unicode ( ▂▃▄▅▆▇█)
pub fn render_ascii_bars(bands: &[f32], max_height: usize) -> String {
    const BLOCKS: [char; 8] = [' ', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

    if max_height <= 1 {
        // Однострочный режим
        return bands
            .iter()
            .map(|&val| {
                let idx = ((val * (BLOCKS.len() - 1) as f32).round() as usize).min(BLOCKS.len() - 1);
                BLOCKS[idx]
            })
            .collect();
    }

    // Многострочный режим
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
