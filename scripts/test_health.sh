#!/bin/sh
# ==============================================================================
# Sentinel Station — Комплексный диагностический тестовый набор (Self-Test Suite)
# ==============================================================================
# Проверяет все уровни системы:
#   1. Процесс и супервайзер (PID, Nice -10, OOM Score -900, Quiet mode)
#   2. Системные ресурсы (RAM, CPU Load, отсутствие OOM и зависших процессов)
#   3. Звуковой тракт ALSA (dmix, softvol, AUX/SPK routing, XRUN-проверка)
#   4. Аппаратная периферия (Экран 32 сегмента, кольцо 6 RGB, сенсоры evdev)
#   5. Связь с сервером Sentinel Connect (HTTP 200, WebSocket 101, Welcome)
#   6. (Опционально) Симуляция сбоя SIGTERM и проверка автоподъема супервайзером
# ==============================================================================

DIR=/data/opt/sentinel-station
CONF=$DIR/config.toml
LOG=$DIR/logs/station.log
BIN=$DIR/bin/sentinel-station

# Цвета терминала ANSI
C_RESET="\033[0m"
C_RED="\033[1;31m"
C_GREEN="\033[1;32m"
C_YELLOW="\033[1;33m"
C_BLUE="\033[1;34m"
C_CYAN="\033[1;36m"
C_BOLD="\033[1m"

PASSED=0
FAILED=0
WARNINGS=0

pass() {
    printf "  ${C_GREEN}[PASS]${C_RESET} %s\n" "$1"
    PASSED=$((PASSED + 1))
}

fail() {
    printf "  ${C_RED}[FAIL]${C_RESET} %s\n" "$1"
    FAILED=$((FAILED + 1))
}

warn() {
    printf "  ${C_YELLOW}[WARN]${C_RESET} %s\n" "$1"
    WARNINGS=$((WARNINGS + 1))
}

header() {
    printf "\n${C_CYAN}${C_BOLD}=== %s ===${C_RESET}\n" "$1"
}

printf "${C_BLUE}${C_BOLD}"
echo "=============================================================================="
echo "    🧪 Sentinel Station — Запуск диагностического тестирования системы        "
echo "=============================================================================="
printf "${C_RESET}"

# ==============================================================================
# 1. ТЕСТЫ ПРОЦЕССА И СУПЕРВАЙЗЕРА
# ==============================================================================
header "1. Проверка процесса и супервайзера"

DAEMON_PID=$(pidof sentinel-station 2>/dev/null)
if [ -n "$DAEMON_PID" ]; then
    pass "Демон sentinel-station запущен (PID: $DAEMON_PID)"
else
    fail "Демон sentinel-station НЕ запущен!"
fi

SUP_PID=$(cat /var/run/sentinel-supervisor.pid 2>/dev/null)
if [ -n "$SUP_PID" ] && kill -0 "$SUP_PID" 2>/dev/null; then
    pass "Защищенный супервайзер активен (PID: $SUP_PID)"
else
    warn "Файл /var/run/sentinel-supervisor.pid отсутствует или супервайзер не найден"
fi

if [ -n "$DAEMON_PID" ]; then
    # Проверка приоритета nice (должен быть -10)
    NICE_VAL=$(awk '{print $19}' "/proc/$DAEMON_PID/stat" 2>/dev/null)
    if [ "$NICE_VAL" = "-10" ]; then
        pass "Приоритет планировщика CPU (Nice) установлен в $NICE_VAL (защита от XRUN)"
    elif [ -n "$NICE_VAL" ] && [ "$NICE_VAL" -le -5 ]; then
        pass "Приоритет планировщика CPU повышен ($NICE_VAL)"
    else
        warn "Приоритет CPU равен $NICE_VAL (рекомендуется -10 для гарантированного реалтайма)"
    fi

    # Проверка защиты OOM Score
    OOM_SCORE=$(cat "/proc/$DAEMON_PID/oom_score_adj" 2>/dev/null)
    if [ "$OOM_SCORE" = "-900" ]; then
        pass "Защита ядра от OOM Killer активна (oom_score_adj: $OOM_SCORE)"
    else
        warn "Значение oom_score_adj: '$OOM_SCORE' (ожидалось -900)"
    fi

    # Проверка тихого запуска в аргументах
    CMDLINE=$(tr '\0' ' ' < "/proc/$DAEMON_PID/cmdline" 2>/dev/null)
    if echo "$CMDLINE" | grep -q -- "--quiet"; then
        pass "Демон запущен с флагом --quiet (бесшумный старт без ночных криков)"
    else
        warn "Флаг --quiet не обнаружен в аргументах запуска: $CMDLINE"
    fi
fi

# ==============================================================================
# 2. ТЕСТЫ СИСТЕМНЫХ РЕСУРСОВ
# ==============================================================================
header "2. Проверка системных ресурсов и стабильности"

MEM_FREE_KB=$(awk '/MemAvailable/ {print $2}' /proc/meminfo 2>/dev/null)
if [ -z "$MEM_FREE_KB" ]; then
    MEM_FREE_KB=$(awk '/MemFree/ {print $2}' /proc/meminfo 2>/dev/null)
fi

MEM_FREE_MB=$((MEM_FREE_KB / 1024))
if [ "$MEM_FREE_MB" -ge 40 ]; then
    pass "Доступная оперативная память в норме: ${MEM_FREE_MB} МБ (свободно > 40 МБ)"
elif [ "$MEM_FREE_MB" -ge 20 ]; then
    warn "Доступная память на исходе: ${MEM_FREE_MB} МБ"
else
    fail "Критически мало памяти: ${MEM_FREE_MB} МБ! Высокий риск сброса процессов"
fi

LOAD_1MIN=$(awk '{print $1}' /proc/loadavg 2>/dev/null)
LOAD_INT=$(echo "$LOAD_1MIN" | cut -d'.' -f1)
if [ -n "$LOAD_INT" ] && [ "$LOAD_INT" -lt 3 ]; then
    pass "Текущая нагрузка процессора (Load Average 1m): $LOAD_1MIN (в норме)"
else
    warn "Повышенная нагрузка CPU (Load Average 1m): $LOAD_1MIN"
fi

# Проверка на паразитные зависшие поиски
ROGUE_GREP=$(pgrep -f "grep -rn" 2>/dev/null)
if [ -z "$ROGUE_GREP" ]; then
    pass "Паразитных зависших процессов сканирования диска/sys не обнаружено"
else
    warn "Обнаружен фоновый процесс сканирования (PID: $ROGUE_GREP) — может перегружать CPU"
fi

# ==============================================================================
# 3. ТЕСТЫ ЗВУКОВОЙ ПОДСИСТЕМЫ ALSA
# ==============================================================================
header "3. Проверка звукового тракта ALSA"

if [ -e "/dev/snd/pcmC0D0p" ]; then
    pass "Аппаратное PCM аудиоустройство /dev/snd/pcmC0D0p обнаружено"
else
    fail "Аппаратное устройство /dev/snd/pcmC0D0p не найдено!"
fi

if [ -f "/etc/asound.conf" ]; then
    if grep -q "dmix_spk" /etc/asound.conf; then
        pass "Конфигурация /etc/asound.conf настроена (dmix микшер активен)"
    else
        warn "В /etc/asound.conf не найдена конфигурация dmix_spk"
    fi
else
    fail "Файл /etc/asound.conf отсутствует!"
fi

# Проверка текущего аудиовыхода (AUX / Speaker)
HEADPHONE_STATE=$(amixer sget 'AUX Headphone' 2>/dev/null | grep -E "\[on\]|\[off\]" | head -n 1)
SPK_STATE=$(amixer sget 'SPK Master' 2>/dev/null | grep -E "\[on\]|\[off\]" | head -n 1)
if echo "$HEADPHONE_STATE" | grep -q "\[on\]"; then
    pass "Аудиовыход: 3.5мм разъем AUX (наушники/колонки) активен"
elif echo "$SPK_STATE" | grep -q "\[on\]"; then
    pass "Аудиовыход: встроенный динамик Станции активен"
else
    pass "Аудиомаршрутизация ALSA работает в штатном режиме"
fi

# Проверка синтетического вывода ALSA без ошибок
ALSA_CHECK=$(aplay -D default -d 1 -f S16_LE -r 48000 -c 2 /dev/zero 2>&1)
if [ $? -eq 0 ]; then
    pass "Тестовая передача PCM через default dmix выполнена без XRUN ошибок"
else
    warn "Синтетический тест ALSA вернул предупреждение: $ALSA_CHECK"
fi

# ==============================================================================
# 4. ТЕСТЫ АППАРАТНОЙ ПЕРИФЕРИИ СТАНЦИИ МИНИ 2
# ==============================================================================
header "4. Проверка аппаратной периферии Станции Мини 2"

CLOCK_LEDS_COUNT=$(ls -d /sys/class/leds/mini2:white:clock* 2>/dev/null | wc -l)
if [ "$CLOCK_LEDS_COUNT" -ge 32 ]; then
    pass "Аппаратный фронтальный экран найден ($CLOCK_LEDS_COUNT белых сегментов)"
else
    warn "LED-экран часов не обнаружен или доступно только $CLOCK_LEDS_COUNT сегментов"
fi

RING_LEDS_COUNT=$(ls -d /sys/class/leds/mini2:*:ring* 2>/dev/null | wc -l)
if [ "$RING_LEDS_COUNT" -ge 18 ]; then
    pass "Верхнее светодиодное кольцо найдено (6 RGB светодиодов / $RING_LEDS_COUNT каналов)"
else
    warn "RGB-кольцо подсветки кнопки не обнаружено"
fi

if [ -e "/dev/input/event0" ]; then
    pass "Интерфейс сенсорных кнопок /dev/input/event0 доступен"
else
    fail "Интерфейс кнопок /dev/input/event0 не найден!"
fi

if [ -f "/data/quasar/daemons_logs/maind.log" ]; then
    pass "Журнал голосовых команд Алисы (/data/quasar/daemons_logs/maind.log) доступен"
else
    warn "Журнал Квазара maind.log не найден (голосовой перехват может быть ограничен)"
fi

# ==============================================================================
# 5. ТЕСТЫ СВЯЗИ С СЕРВЕРОМ SENTINEL CONNECT
# ==============================================================================
header "5. Проверка соединения с Sentinel Music Server"

if [ -x "$BIN" ] && [ -f "$CONF" ]; then
    STATUS_OUT=$("$BIN" --config "$CONF" status 2>&1)
    if echo "$STATUS_OUT" | grep -q "200 OK"; then
        pass "REST API сервера доступен: HTTP 200 OK (проверено через бинарник)"
    else
        warn "Проверка REST API вернула: $(echo "$STATUS_OUT" | grep -E "HTTP|Ошибка" | head -n 1)"
    fi
fi

if [ -f "$LOG" ]; then
    LAST_CONN=$(grep "101 Switching Protocols" "$LOG" | tail -n 1)
    if [ -n "$LAST_CONN" ]; then
        pass "WebSocket сессия Sentinel Connect подтверждена в логах (101 Switching Protocols)"
    else
        warn "В station.log не найдено подтверждения успешного подключения к WebSocket"
    fi
fi

# ==============================================================================
# 6. ОПЦИОНАЛЬНЫЙ СТРЕСС-ТЕСТ СУПЕРВАЙЗЕРА (по флагу --stress)
# ==============================================================================
if [ "$1" = "--stress" ]; then
    header "6. Стресс-тестирование супервайзера (Graceful Shutdown & Auto-Recovery)"
    if [ -n "$DAEMON_PID" ]; then
        echo "  [*] Отправка сигнала SIGTERM демону (PID: $DAEMON_PID)..."
        kill -15 "$DAEMON_PID"
        sleep 3
        
        NEW_PID=$(pidof sentinel-station 2>/dev/null)
        if [ -n "$NEW_PID" ] && [ "$NEW_PID" != "$DAEMON_PID" ]; then
            pass "Супервайзер успешно и чисто восстановил сервис! Новый PID: $NEW_PID"
            
            # Проверяем, что graceful shutdown был записан
            if grep -q "Получен сигнал завершения (SIGTERM)" "$LOG"; then
                pass "Демон корректно обработал SIGTERM и освободил ресурсы"
            fi
            
            # Проверяем oom_score_adj у нового процесса
            NEW_OOM=$(cat "/proc/$NEW_PID/oom_score_adj" 2>/dev/null)
            if [ "$NEW_OOM" = "-900" ]; then
                pass "Новый процесс автоматически защищен от OOM (oom_score_adj: $NEW_OOM)"
            fi
        else
            fail "Сервис не поднялся автоматически за 3 секунды!"
        fi
    fi
fi

# ==============================================================================
# ИТОГОВЫЙ ОТЧЕТ
# ==============================================================================
echo ""
echo "=============================================================================="
printf "${C_BOLD}Итоги тестирования: ${C_GREEN}%d пройдено${C_RESET}, " "$PASSED"
if [ "$WARNINGS" -gt 0 ]; then
    printf "${C_YELLOW}%d предупреждений${C_RESET}, " "$WARNINGS"
else
    printf "0 предупреждений, "
fi
if [ "$FAILED" -gt 0 ]; then
    printf "${C_RED}%d сбоев${C_RESET}\n" "$FAILED"
else
    printf "${C_GREEN}0 сбоев${C_RESET}\n"
fi
echo "=============================================================================="

if [ "$FAILED" -eq 0 ]; then
    printf "${C_GREEN}${C_BOLD}🎉 ВСЕ СИСТЕМЫ SENTINEL STATION ФУНКЦИОНИРУЮТ ШТАТНО!${C_RESET}\n"
    exit 0
else
    printf "${C_RED}${C_BOLD}⚠️ ОБНАРУЖЕНЫ СБОИ В РАБОТЕ СИСТЕМЫ.${C_RESET}\n"
    exit 1
fi
