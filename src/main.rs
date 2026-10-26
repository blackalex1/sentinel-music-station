use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rodio::{OutputStream, OutputStreamHandle};
use tokio::sync::{mpsc, watch};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

mod audio;
mod config;
mod connect;
mod input;
mod tui;
mod visualizer;

use audio::AudioPlayer;
use config::StationConfig;
use connect::{ConnectClient, PlayerCommand, PlayerState};
use input::{HardwareButtonController, VoiceInterceptor};
use tui::TuiApp;
use visualizer::DisplayManager;

#[derive(Parser, Debug)]
#[command(
    name = "sentinel-station",
    author = "Sentinel Music Ecosystem",
    version,
    about = "Нативный демон воспроизведения Sentinel Music для умных колонок (Яндекс.Станция)"
)]
struct Cli {
    /// Путь к файлу конфигурации (по умолчанию: ./config.toml или /data/opt/sentinel-station/config.toml)
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Запуск в режиме фонового демона (по умолчанию)
    Start,

    /// Запуск интерактивного консольного терминала (TUI) через SSH
    Tui,

    /// Воспроизвести конкретный трек по ID для проверки звука
    Play {
        #[arg(value_name = "TRACK_ID")]
        track_id: i64,
    },

    /// Проверить статус подключения к серверу Sentinel Music
    Status,

    /// Показать текущую конфигурацию
    Config,
}

/// Очищает зависшие System V IPC семафоры и сегменты разделяемой памяти ALSA dmix (ipc_key 1024),
/// которые могут остаться после аварийной остановки предыдущего процесса
fn cleanup_stale_alsa_ipc() {
    let _ = std::process::Command::new("sh")
        .args([
            "-c",
            "for s in $(ipcs -s 2>/dev/null | awk '$1==\"0x00000400\"{print $2}'); do ipcrm -s $s 2>/dev/null; done; for m in $(ipcs -m 2>/dev/null | awk '$1==\"0x00000400\"||$1==\"0x00000401\"{print $2}'); do ipcrm -m $m 2>/dev/null; done",
        ])
        .output();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let is_tui = matches!(cli.command, Some(Commands::Tui));

    // Настройка логирования: в интерактивном TUI-режиме перенаправляем логи в файл,
    // чтобы они не перебивали вывод Crossterm/Ratatui в SSH консоли
    if is_tui {
        let log_path = std::env::temp_dir().join("sentinel-station-tui.log");
        if let Ok(log_file) = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)
        {
            let env_filter = EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,sentinel_station=debug"));

            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(log_file)
                .with_ansi(false)
                .with_target(false)
                .init();
        } else {
            tracing_subscriber::fmt()
                .with_writer(std::io::sink)
                .init();
        }
    } else {
        let env_filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("info,sentinel_station=debug"));

        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .init();
    }

    let cfg = StationConfig::load(cli.config.as_deref())
        .context("Ошибка инициализации файла конфигурации")?;
    let config = Arc::new(cfg);

    match cli.command.unwrap_or(Commands::Start) {
        Commands::Config => {
            println!("--- ТЕКУЩАЯ КОНФИГУРАЦИЯ SENTINEL STATION ---");
            println!("{}", toml::to_string_pretty(&*config)?);
            println!("WebSocket URL: {}", config.websocket_url()?);
            Ok(())
        }
        Commands::Status => {
            println!("📡 Проверка подключения к Sentinel Music Server...");
            println!("Сервер: {}", config.server_url);
            println!("Имя устройства: {}", config.device_name);
            println!("ID устройства: {}", config.device_id);

            let client = reqwest::Client::new();
            let health_url = format!("{}/api/system/version", config.server_url.trim_end_matches('/'));
            match client.get(&health_url).timeout(Duration::from_secs(5)).send().await {
                Ok(resp) if resp.status().is_success() => {
                    println!("✅ Сервер доступен и отвечает (HTTP {})", resp.status());
                }
                Ok(resp) => {
                    println!("⚠️ Сервер вернул статус: HTTP {}", resp.status());
                }
                Err(e) => {
                    println!("❌ Ошибка соединения с сервером: {}", e);
                }
            }
            Ok(())
        }
        Commands::Play { track_id } => {
            info!("Тестовый запуск трека ID: {} на звуковом устройстве '{}'", track_id, config.audio_device);
            cleanup_stale_alsa_ipc();
            let (_stream, stream_handle) = OutputStream::try_default()
                .context("Не удалось открыть звуковое устройство ALSA/Audio")?;

            let (cmd_tx, cmd_rx) = mpsc::channel(32);
            let (state_tx, mut state_rx) = watch::channel(PlayerState::default());
            let sample_buffer = Arc::new(Mutex::new(Vec::with_capacity(4096)));

            if config.visualizer_enabled {
                let sample_buf_clone = sample_buffer.clone();
                let state_rx_clone = state_tx.subscribe();
                let brightness = config.display_brightness;
                let tz_offset = config.timezone_offset_hours;
                let (_, dummy_rx) = mpsc::channel(4);
                tokio::spawn(async move {
                    let display_mgr = DisplayManager::new(
                        sample_buf_clone,
                        state_rx_clone,
                        dummy_rx,
                        brightness,
                        tz_offset,
                    );
                    display_mgr.run_loop().await;
                });
            }

            let mut player = AudioPlayer::new(
                config.clone(),
                stream_handle,
                cmd_rx,
                state_tx,
                sample_buffer,
            )?;

            cmd_tx.send(PlayerCommand::PlayTrack {
                track_id,
                title: format!("Тестовый трек #{}", track_id),
                artist: "Sentinel Music".to_string(),
                duration_ms: 0,
                position_ms: 0,
                auto_play: true,
                queue: Vec::new(),
            }).await?;

            tokio::spawn(async move {
                player.run_loop().await;
            });

            println!("Воспроизведение запущено. Нажмите Ctrl+C для остановки.");
            while state_rx.changed().await.is_ok() {
                let st = state_rx.borrow().clone();
                if st.is_playing {
                    let pos_s = st.position_ms / 1000;
                    print!("\r▶ Играет... {:02}:{:02}", pos_s / 60, pos_s % 60);
                    std::io::Write::flush(&mut std::io::stdout())?;
                }
            }
            Ok(())
        }
        Commands::Start => {
            cleanup_stale_alsa_ipc();
            let (_stream, stream_handle) = OutputStream::try_default()
                .context("Не удалось открыть звуковое устройство ALSA/Audio")?;
            run_daemon(config, stream_handle).await
        }
        Commands::Tui => {
            run_tui_mode(config).await
        }
    }
}

/// Запуск в режиме фонового демона
async fn run_daemon(config: Arc<StationConfig>, stream_handle: OutputStreamHandle) -> Result<()> {
    info!("Запуск демона Sentinel Station v{} (ALSA Native)", env!("CARGO_PKG_VERSION"));
    info!("Имя устройства в Sentinel Connect: '{}'", config.device_name);
    info!("Аудиовыход: '{}'", config.audio_device);

    let (cmd_tx, cmd_rx) = mpsc::channel(64);
    let (state_tx, state_rx) = watch::channel(PlayerState::default());
    let (is_connected_tx, _) = watch::channel(false);
    let (display_tx, display_rx) = mpsc::channel(32);

    let sample_buffer = Arc::new(Mutex::new(Vec::with_capacity(4096)));

    // 1. Инициализация аудиодвижка
    let mut player = AudioPlayer::new(
        config.clone(),
        stream_handle,
        cmd_rx,
        state_tx,
        sample_buffer.clone(),
    )?;

    // 2. Фоновый поток вывода на аппаратный LED-экран Станции Мини 2
    if config.visualizer_enabled {
        let sample_buf_clone = sample_buffer.clone();
        let state_rx_clone = state_rx.clone();
        let brightness = config.display_brightness;
        let tz_offset = config.timezone_offset_hours;
        tokio::spawn(async move {
            let display_mgr = DisplayManager::new(
                sample_buf_clone,
                state_rx_clone,
                display_rx,
                brightness,
                tz_offset,
            );
            display_mgr.run_loop().await;
        });
    }

    // 3. Фоновый контроллер сенсорных кнопок на корпусе (Сенсоры AW9610X на Станции Мини 2)
    if config.buttons_enabled {
        let button_controller = HardwareButtonController::new(
            &config.input_device_path,
            cmd_tx.clone(),
            state_rx.clone(),
            display_tx.clone(),
        );
        button_controller.start();
    }

    // 3.1. Фоновый перехватчик голосовых команд Алисы (мониторинг лога Quasar / maind)
    let voice_interceptor = VoiceInterceptor::new(
        cmd_tx.clone(),
        state_rx.clone(),
        display_tx.clone(),
    );
    voice_interceptor.start();

    // 4. Запуск аудиодвижка в фоне
    let player_handle = tokio::spawn(async move {
        player.run_loop().await;
    });

    // 5. Запуск супервайзера постоянного подключения к серверу
    let mut client = ConnectClient::new(
        config.clone(),
        cmd_tx,
        state_rx,
        is_connected_tx,
    );

    tokio::select! {
        _ = client.run_supervisor() => {
            info!("Супервайзер подключения завершил работу.");
        }
        _ = player_handle => {
            error!("Аудиоплеер непредвиденно завершил работу.");
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Получен сигнал завершения (SIGINT/Ctrl+C). Остановка Sentinel Station...");
        }
    }

    Ok(())
}

/// Запуск в интерактивном TUI-режиме для SSH консоли (без создания второго аудиоустройства)
async fn run_tui_mode(config: Arc<StationConfig>) -> Result<()> {
    // В режиме TUI подключаемся как отдельное удаленное устройство-контроллер (remote),
    // чтобы не конфликтовать с работающим фоновым демоном плеера и не занимать ALSA.
    let mut tui_cfg = (*config).clone();
    tui_cfg.device_id = format!("{}-tui", config.device_id);
    tui_cfg.device_name = format!("{} (TUI)", config.device_name);
    tui_cfg.device_type = "remote".to_string();
    let tui_config = Arc::new(tui_cfg);

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<PlayerCommand>(64);
    let (state_tx, state_rx) = watch::channel(PlayerState::default());
    let (is_connected_tx, is_connected_rx) = watch::channel(false);
    let sample_buffer = Arc::new(Mutex::new(Vec::with_capacity(4096)));

    // Клиент Sentinel Connect в режиме remote с приемом состояния основной колонки
    let mut client = ConnectClient::new(
        tui_config.clone(),
        cmd_tx.clone(),
        state_rx.clone(),
        is_connected_tx,
    ).with_remote_state_tx(state_tx);

    tokio::spawn(async move {
        client.run_supervisor().await;
    });

    // Фоновая пересылка команд TUI через REST API целевой колонке
    let http_client = reqwest::Client::new();
    let target_device_id = config.device_id.clone();
    let server_url = config.server_url.clone();
    let auth_token = config.auth_token.clone();
    tokio::spawn(async move {
        let command_endpoint = format!("{}/api/remote/command", server_url.trim_end_matches('/'));
        while let Some(cmd) = cmd_rx.recv().await {
            let (cmd_str, payload) = match cmd {
                PlayerCommand::TogglePlay => ("TOGGLE_PLAY", serde_json::json!({})),
                PlayerCommand::Play => ("PLAY", serde_json::json!({})),
                PlayerCommand::Pause => ("PAUSE", serde_json::json!({})),
                PlayerCommand::Stop => ("STOP", serde_json::json!({})),
                PlayerCommand::Next => ("NEXT", serde_json::json!({})),
                PlayerCommand::Prev => ("PREV", serde_json::json!({})),
                PlayerCommand::SetVolume { volume } => ("SET_VOLUME", serde_json::json!({ "volume": volume })),
                PlayerCommand::Seek { position_ms } => ("SEEK", serde_json::json!({ "position_ms": position_ms })),
                PlayerCommand::ToggleShuffle => ("SHUFFLE", serde_json::json!({})),
                PlayerCommand::SetShuffle { enabled } => ("SHUFFLE", serde_json::json!({ "enabled": enabled })),
                PlayerCommand::SetRepeat { mode } => ("REPEAT", serde_json::json!({ "mode": mode })),
                PlayerCommand::PlayTrack { track_id, .. } => ("PLAY_TRACK", serde_json::json!({ "track_id": track_id })),
            };

            let mut req = http_client.post(&command_endpoint)
                .json(&serde_json::json!({
                    "target_device_id": target_device_id,
                    "command": cmd_str,
                    "payload": payload,
                }));
            if !auth_token.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", auth_token));
            }
            let _ = req.send().await;
        }
    });

    // Установка хука паники для гарантированного восстановления терминала
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        original_hook(panic_info);
    }));

    // Инициализация терминала Crossterm
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::Hide
    )?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;
    terminal.clear()?;

    let mut tui_app = TuiApp::new(
        tui_config,
        cmd_tx,
        state_rx,
        is_connected_rx,
        sample_buffer,
    );

    let res = tui_app.run(&mut terminal).await;

    // Восстановление терминала
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;

    res
}
