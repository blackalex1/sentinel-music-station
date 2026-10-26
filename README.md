# Sentinel Station (Native Root / Rust)

Нативный клиент и фоновый демон **Sentinel Music** для умных колонок (Яндекс.Станция Мини 1, Мини 2 и других устройств с root-доступом), вдохновленный проектом [klockky/spotiya](https://github.com/klockky/spotiya), но работающий с вашей собственной независимой стриминговой экосистемой **Sentinel Music**.

---

## ⚡ Особенности и возможности

* **Прямой вывод в ALSA:** звук воспроизводится напрямую через подсистему Linux ALSA (`libasound`), гарантируя минимальную задержку и чистое звучание.
* **Поддержка Hi-Fi форматов:** встроенный декодер на чистом Rust (Symphonia) без внешних C-библиотек декодирует **FLAC (Lossless)**, **MP3**, **AAC** и **WAV**.
* **Интеграция с Sentinel Connect:** колонка регистрируется на вашем сервере как удаленный плеер. Вы можете управлять воспроизведением, громкостью и треками с телефона (Sentinel Android), десктоп-клиента или веб-интерфейса.
* **Аппаратная визуализация на LED-экране (Яндекс Станция Мини 2):**
  * Встроенный анализатор спектра на базе быстрого преобразования Фурье (**FFT**).
  * Динамический эквалайзер с физикой (Attack/Decay) выводится прямо на лицевую матрицу часов колонки во время звучания треков!
* **Интерактивный терминальный интерфейс (TUI):**
  * При подключении по SSH доступен режим `sentinel-station tui` (аналог `spotiya tui`).
  * Отображает обложку/название трека, красивый прыгающий спектрограф в реальном времени, слайдер громкости и таймлайн.
* **Режим супервайзера (Daemon):**
  * Автоматический перезапуск потока и переподключение к серверу с экспоненциальной задержкой при обрывах Wi-Fi.

---

## 🏛 Архитектура

```text
  [ Sentinel Android / Desktop / Web ]
                   │
                   │ (1. Выбор колонки: "Играть на Станции")
                   ▼
       [ Sentinel Music Server ]
                   │
                   │ (2. WebSocket: /api/v1/remote/ws)
                   ▼
 ┌─────────────────────────────────────────────────────────┐
 │ Яндекс.Станция (/data/opt/sentinel-station/)             │
 │                                                         │
 │  [ConnectClient] ──> [AudioPlayer] ──> [ALSA /dev/snd]  │
 │         │                  │                            │
 │         │ (Статус)         ▼ (PCM сэмплы)               │
 │         ▼           [SpectrumAnalyzer (FFT)]            │
 │     [Сервер]               │                            │
 │                   ┌────────┴────────┐                   │
 │                   ▼                 ▼                   │
 │            [LED Matrix]       [TUI SSH Console]         │
 │          (Экран часов)        (Эквалайзер в консоли)    │
 └─────────────────────────────────────────────────────────┘
```

---

## 🛠️ Сборка

### 1. Локальная проверка и сборка на ПК (для тестов)
```bash
cd station
cargo build --release
```

### 2. Кросс-компиляция под ARMv7 (для Яндекс Станции)

**Вариант А: Через утилиту `cross` (рекомендуется):**
```bash
cargo install cross --git https://github.com/cross-rs/cross
cross build --target armv7-unknown-linux-musleabihf --release
```

**Вариант Б: Через Docker (без установки тулчейнов):**
```bash
docker build -f scripts/Dockerfile.armv7 -t sentinel-station-builder .
docker create --name station-tmp sentinel-station-builder
docker cp station-tmp:/sentinel-station ./sentinel-station-armv7
docker rm station-tmp
```

---

## 📦 Установка на Яндекс.Станцию

1. Подключитесь к рутованной колонке по SSH:
   ```bash
   ssh root@<IP_ВАШЕЙ_КОЛОНКИ>
   ```

2. Скопируйте собранный бинарник и файлы на колонку:
   ```bash
   scp target/armv7-unknown-linux-musleabihf/release/sentinel-station root@<IP_КОЛОНКИ>:/data/
   scp scripts/install.sh root@<IP_КОЛОНКИ>:/data/
   ```

3. Запустите установку на колонке:
   ```bash
   chmod +x /data/install.sh
   sh /data/install.sh
   ```

Скрипт автоматически:
* Разместит файлы в защищенном энергонезависимом разделе `/data/opt/sentinel-station/`.
* Настроит службу автозапуска (`systemd` или `/etc/init.d/S99sentinel-station`).
* Создаст конфигурационный файл `/data/opt/sentinel-station/config.toml`.

---

## ⚙️ Настройка `config.toml`

Откройте файл `/data/opt/sentinel-station/config.toml` и укажите параметры вашей сети:

```toml
# URL вашего Sentinel Music Server
server_url = "http://192.168.1.100:8000"

# Токен авторизации (API токен из профиля веб-плеера)
auth_token = "ВАШ_ТОКЕН"

# Имя колонки в списке устройств Sentinel Connect
device_name = "Яндекс.Станция (Гостиная)"

# Аудиовыход ALSA (по умолчанию "default")
audio_device = "default"

# Начальная громкость (0.0 .. 1.0)
initial_volume = 0.7

# Включить FFT-спектрограмму
visualizer_enabled = true

# Для Станции Мини 2 с часами: устройство вывода LED-матрицы
# led_matrix_path = "/dev/matrix"
```

После изменения конфигурации перезапустите службу:
```bash
# Для systemd:
systemctl restart sentinel-station

# Для init.d:
/etc/init.d/S99sentinel-station restart
```

---

## 🎮 Использование и консольный интерфейс (TUI)

Вы можете в любой момент подключиться к колонке по SSH и запустить интерактивный TUI-терминал:

```bash
/data/opt/sentinel-station/bin/sentinel-station tui
```

### Горячие клавиши в TUI:
* `[Пробел]` — Пауза / Возобновление воспроизведения.
* `[+]` / `[-]` — Регулировка громкости ALSA.
* `[M]` — Заглушить / Включить звук (Mute).
* `[Q]` / `[Esc]` — Выход из TUI обратно в командную строку Linux (демон в фоне продолжит играть музыку).
