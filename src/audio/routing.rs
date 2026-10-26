use tracing::info;

/// Проверяет, вставлен ли 3.5мм аудиокабель (AUX / наушники) в разъем колонки
pub fn is_headphone_plugged() -> bool {
    let output = std::process::Command::new("amixer")
        .args(["-c", "0", "cget", "iface=CARD,name='Headphones Jack'"])
        .output();

    if let Ok(out) = output {
        let text = String::from_utf8_lossy(&out.stdout);
        text.contains("values=on")
    } else {
        false
    }
}

/// Синхронизирует аппаратные переключатели ALSA между встроенным динамиком и 3.5мм разъемом AUX:
/// - "auto" (по умолчанию): если вставлен Jack -> AUX on, SPK off; если вынут -> AUX off, SPK on
/// - "both": одновременно включены и динамик Станции, и 3.5мм разъем AUX
/// - "speaker": только встроенный динамик
/// - "aux": только разъем AUX
pub fn sync_hardware_routing(last_state: &mut Option<bool>, routing_mode: &str) {
    if routing_mode != "auto" {
        if last_state.is_some() {
            return;
        }
        *last_state = Some(true);
        match routing_mode {
            "speaker" => {
                info!("📢 Режим звука: только встроенный динамик");
                let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "AUX Headphone", "off"]).output();
                let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "SPK Master", "on"]).output();
            }
            "aux" => {
                info!("🎧 Режим звука: только 3.5мм разъем (AUX)");
                let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "AUX Headphone", "on"]).output();
                let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "SPK Master", "off"]).output();
            }
            "both" => {
                info!("🔊 Режим звука: параллельно встроенный динамик и 3.5мм разъем (AUX)");
                let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "AUX Headphone", "on"]).output();
                let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "SPK Master", "on"]).output();
            }
            _ => {}
        }
        return;
    }

    let plugged = is_headphone_plugged();
    if Some(plugged) == *last_state {
        return;
    }

    *last_state = Some(plugged);
    if plugged {
        info!("🎧 Обнаружен кабель 3.5мм (AUX): звук перенаправлен в разъем AUX, внутренний динамик выключен");
        let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "AUX Headphone", "on"]).output();
        let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "SPK Master", "off"]).output();
    } else {
        info!("📢 Кабель 3.5мм отключен: воспроизведение переключено на встроенный динамик Станции");
        let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "AUX Headphone", "off"]).output();
        let _ = std::process::Command::new("amixer").args(["-c", "0", "sset", "SPK Master", "on"]).output();
    }
}

/// Устанавливает громкость через аппаратный супервайзер hardvold:
/// hardvold транслирует значение MasterHW в физические усилители SPK Master и AUX DAC с точными кривыми dB
pub fn set_hardware_volume(volume: f32) {
    let percent = (volume.clamp(0.0, 1.0) * 100.0).round() as u32;
    let pct_str = format!("{}%", percent);

    let apply_mixers = move || {
        let _ = std::process::Command::new("amixer")
            .args(["-c", "0", "sset", "MasterHW", &pct_str])
            .output();
        let _ = std::process::Command::new("amixer")
            .args(["-c", "0", "sset", "Master", "100%"])
            .output();
    };

    if tokio::runtime::Handle::try_current().is_ok() {
        tokio::task::spawn_blocking(apply_mixers);
    } else {
        apply_mixers();
    }
}
