#!/usr/bin/env bash
# ==============================================================================
# Скрипт кросс-компиляции Sentinel Station под ARMv7 (Яндекс Станция)
# ==============================================================================
set -e

TARGET="armv7-unknown-linux-musleabihf"
echo "=== Кросс-компиляция Sentinel Station под $TARGET ==="

# Проверка наличия утилиты cross
if command -v cross >/dev/null 2>&1; then
    echo "[*] Использование утилиты 'cross'..."
    cross build --target "$TARGET" --release
else
    echo "[!] Утилита 'cross' не найдена. Установите ее командой:"
    echo "    cargo install cross --git https://github.com/cross-rs/cross"
    echo ""
    echo "[*] Попытка сборки через стандартный cargo (требуется установленный arm-linux-musleabihf тулчейн)..."
    rustup target add "$TARGET" || true
    cargo build --target "$TARGET" --release
fi

OUTPUT_BIN="target/$TARGET/release/sentinel-station"
if [ -f "$OUTPUT_BIN" ]; then
    echo ""
    echo "[+] Успешно собран бинарник: $OUTPUT_BIN"
    ls -lh "$OUTPUT_BIN"
    echo ""
    echo "Для отправки на колонку по SCP используйте:"
    echo "    scp $OUTPUT_BIN root@<IP_КОЛОНКИ>:/data/opt/sentinel-station/bin/sentinel-station"
fi
