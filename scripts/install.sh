#!/bin/sh
# ==============================================================================
# Sentinel Station — Скрипт установки на рутованную Яндекс.Станцию
# ==============================================================================
set -e

INSTALL_DIR="/data/opt/sentinel-station"
BIN_NAME="sentinel-station"
SERVICE_NAME="sentinel-station"

echo "=== Установка Sentinel Station Native Daemon ==="

# 1. Проверка прав root
if [ "$(id -u)" -ne 0 ]; then
    echo "[-] Ошибка: скрипт должен выполняться с правами root на колонке!" >&2
    exit 1
fi

# 2. Создание рабочей директории в энергонезависимом разделе /data
echo "[*] Создание директории: $INSTALL_DIR"
mkdir -p "$INSTALL_DIR"
mkdir -p "$INSTALL_DIR/bin"
mkdir -p "$INSTALL_DIR/logs"

# 3. Копирование бинарника
if [ -f "./$BIN_NAME" ]; then
    cp -f "./$BIN_NAME" "$INSTALL_DIR/bin/$BIN_NAME"
elif [ -f "./target/armv7-unknown-linux-musleabihf/release/$BIN_NAME" ]; then
    cp -f "./target/armv7-unknown-linux-musleabihf/release/$BIN_NAME" "$INSTALL_DIR/bin/$BIN_NAME"
else
    echo "[-] Файл $BIN_NAME не найден в текущей папке."
    echo "[*] Если вы скачиваете с GitHub релиза, используйте: wget/curl"
fi

if [ -f "$INSTALL_DIR/bin/$BIN_NAME" ]; then
    chmod +x "$INSTALL_DIR/bin/$BIN_NAME"
    echo "[+] Бинарник установлен и права на исполнение выданы."
fi

# 4. Создание файла конфигурации, если его еще нет
if [ ! -f "$INSTALL_DIR/config.toml" ]; then
    if [ -f "./config.example.toml" ]; then
        cp "./config.example.toml" "$INSTALL_DIR/config.toml"
    else
        cat << 'EOF' > "$INSTALL_DIR/config.toml"
server_url = "http://192.168.1.100:8000"
auth_token = ""
device_name = "Яндекс.Станция (Sentinel)"
audio_device = "default"
initial_volume = 0.7
visualizer_enabled = true
auto_reconnect = true
ping_interval_secs = 15
EOF
    fi
    echo "[+] Создан файл конфигурации: $INSTALL_DIR/config.toml"
    echo "[!] ВНИМАНИЕ: Отредактируйте $INSTALL_DIR/config.toml и укажите IP вашего сервера и токен!"
fi

# 5. Настройка автозапуска
if [ -d "/etc/systemd/system" ]; then
    echo "[*] Обнаружен systemd. Создание службы..."
    cat << EOF > /etc/systemd/system/${SERVICE_NAME}.service
[Unit]
Description=Sentinel Station Native Audio Daemon
After=network.target sound.target

[Service]
Type=simple
WorkingDirectory=$INSTALL_DIR
ExecStart=$INSTALL_DIR/bin/$BIN_NAME start --config $INSTALL_DIR/config.toml
Restart=always
RestartSec=5
StandardOutput=append:$INSTALL_DIR/logs/station.log
StandardError=append:$INSTALL_DIR/logs/station.log

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable ${SERVICE_NAME}
    systemctl restart ${SERVICE_NAME}
    echo "[+] Служба systemd успешно активирована и запущена!"

elif [ -d "/etc/init.d" ]; then
    echo "[*] Обнаружен SysVinit / OpenWrt / Buildroot init.d..."
    cat << EOF > /etc/init.d/S99${SERVICE_NAME}
#!/bin/sh
DAEMON="$INSTALL_DIR/bin/$BIN_NAME"
CONFIG="$INSTALL_DIR/config.toml"
LOG="$INSTALL_DIR/logs/station.log"

case "\$1" in
    start)
        echo "Запуск $SERVICE_NAME..."
        start-stop-daemon -S -b -m -p /var/run/${SERVICE_NAME}.pid -x \$DAEMON -- start --config \$CONFIG >> \$LOG 2>&1
        ;;
    stop)
        echo "Остановка $SERVICE_NAME..."
        start-stop-daemon -K -p /var/run/${SERVICE_NAME}.pid
        ;;
    restart)
        \$0 stop
        sleep 1
        \$0 start
        ;;
    *)
        echo "Использование: \$0 {start|stop|restart}"
        exit 1
        ;;
esac
exit 0
EOF
    chmod +x /etc/init.d/S99${SERVICE_NAME}
    /etc/init.d/S99${SERVICE_NAME} restart
    echo "[+] Скрипт автозапуска /etc/init.d/S99${SERVICE_NAME} создан и запущен!"
fi

echo ""
echo "=============================================================================="
echo "🎉 Sentinel Station успешно установлена!"
echo "Для запуска интерактивного TUI монитора (эквалайзер, статус) выполните:"
echo "    $INSTALL_DIR/bin/$BIN_NAME tui --config $INSTALL_DIR/config.toml"
echo "=============================================================================="
