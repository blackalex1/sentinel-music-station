#!/bin/sh
# ==============================================================================
# Sentinel Station — Интерактивная витрина LED-анимаций для Яндекс Станции Мини 2
# ==============================================================================
# Позволяет наглядно просмотреть все доступные варианты анимаций на физическом
# экране (32 белых сегмента) и верхнем RGB-кольце (6 светодиодов).
# ==============================================================================

BRIGHTNESS=120
BASE_CLOCK="/sys/class/leds"

# Цвета ANSI для меню
C_CYAN="\033[1;36m"
C_GREEN="\033[1;32m"
C_YELLOW="\033[1;33m"
C_MAGENTA="\033[1;35m"
C_RESET="\033[0m"

# Управление одиночным белым сегментом часов
set_seg() {
    echo "${2:-$BRIGHTNESS}" > "/sys/class/leds/mini2:white:clock$1/brightness" 2>/dev/null
}

clear_seg() {
    echo 0 > "/sys/class/leds/mini2:white:clock$1/brightness" 2>/dev/null
}

# Управление светодиодом кольца (1..6) RGB (0..255)
set_ring() {
    echo "${2:-0}" > "/sys/class/leds/mini2:red:ring$1/brightness" 2>/dev/null
    echo "${3:-0}" > "/sys/class/leds/mini2:green:ring$1/brightness" 2>/dev/null
    echo "${4:-0}" > "/sys/class/leds/mini2:blue:ring$1/brightness" 2>/dev/null
}

set_ring_all() {
    for idx in 1 2 3 4 5 6; do
        set_ring "$idx" "$1" "$2" "$3"
    done
}

clear_ring() {
    set_ring_all 0 0 0
}

clear_clock() {
    for f in /sys/class/leds/mini2:white:clock*/brightness; do
        echo 0 > "$f" 2>/dev/null
    done
}

clear_all() {
    clear_clock
    clear_ring
}

# Отрисовка символа на указанном разряде (1..4)
draw_char() {
    d=$1
    ch=$2
    b=${3:-$BRIGHTNESS}

    # Сначала гасим 7 сегментов этого разряда
    for s in A B C D E F G; do
        clear_seg "${s}${d}"
    done

    case "$ch" in
        '0') for s in A B C D E F; do set_seg "${s}${d}" "$b"; done ;;
        '1') for s in B C; do set_seg "${s}${d}" "$b"; done ;;
        '2') for s in A B G E D; do set_seg "${s}${d}" "$b"; done ;;
        '3') for s in A B G C D; do set_seg "${s}${d}" "$b"; done ;;
        '4') for s in F G B C; do set_seg "${s}${d}" "$b"; done ;;
        '5') for s in A F G C D; do set_seg "${s}${d}" "$b"; done ;;
        '6') for s in A F G E C D; do set_seg "${s}${d}" "$b"; done ;;
        '7') for s in A B C; do set_seg "${s}${d}" "$b"; done ;;
        '8') for s in A B C D E F G; do set_seg "${s}${d}" "$b"; done ;;
        '9') for s in A B C D F G; do set_seg "${s}${d}" "$b"; done ;;
        'S'|'s') for s in A F G C D; do set_seg "${s}${d}" "$b"; done ;;
        'E'|'e') for s in A F G E D; do set_seg "${s}${d}" "$b"; done ;;
        'N'|'n') for s in E G C; do set_seg "${s}${d}" "$b"; done ;;
        'T'|'t') for s in F G E D; do set_seg "${s}${d}" "$b"; done ;;
        'I'|'i') for s in B C; do set_seg "${s}${d}" "$b"; done ;;
        'L'|'l') for s in F E D; do set_seg "${s}${d}" "$b"; done ;;
        'O'|'o') for s in C D E G; do set_seg "${s}${d}" "$b"; done ;;
        'F'|'f') for s in A F G E; do set_seg "${s}${d}" "$b"; done ;;
        'P'|'p') for s in A B G E F; do set_seg "${s}${d}" "$b"; done ;;
        'A'|'a') for s in A B C E F G; do set_seg "${s}${d}" "$b"; done ;;
        'U'|'u') for s in F E D C B; do set_seg "${s}${d}" "$b"; done ;;
        'Y'|'y') for s in F G B C D; do set_seg "${s}${d}" "$b"; done ;;
        'M'|'m') for s in A E C; do set_seg "${s}${d}" "$b"; done ;;
        'C'|'c') for s in A F E D; do set_seg "${s}${d}" "$b"; done ;;
        '-') set_seg "G${d}" "$b" ;;
        '=') set_seg "G${d}" "$b"; set_seg "D${d}" "$b" ;;
        '>') set_seg "A${d}" "$b"; set_seg "B${d}" "$b"; set_seg "G${d}" "$b"; set_seg "C${d}" "$b" ;;
        '<') set_seg "A${d}" "$b"; set_seg "F${d}" "$b"; set_seg "G${d}" "$b"; set_seg "E${d}" "$b" ;;
        ' ') ;;
    esac
}

# Отрисовка 4-символьного слова
draw_word() {
    w="$1"
    b=${2:-$BRIGHTNESS}
    draw_char 1 "$(echo "$w" | cut -c1)" "$b"
    draw_char 2 "$(echo "$w" | cut -c2)" "$b"
    draw_char 3 "$(echo "$w" | cut -c3)" "$b"
    draw_char 4 "$(echo "$w" | cut -c4)" "$b"
}

# ==============================================================================
# 1. Спиннер / Радар (Вращение по контуру каждого разряда + кольцо)
# ==============================================================================
anim_spinner() {
    printf "${C_GREEN}▶ [1] Спиннер / Радар (круговое вращение по разрядам + кольцо)${C_RESET}\n"
    clear_all
    for cycle in $(seq 1 5); do
        ring=1
        for s in A B C D E F; do
            set_seg "${s}1"
            set_seg "${s}2"
            set_seg "${s}3"
            set_seg "${s}4"
            clear_ring
            set_ring "$ring" 180 0 255
            usleep 75000
            clear_seg "${s}1"
            clear_seg "${s}2"
            clear_seg "${s}3"
            clear_seg "${s}4"
            ring=$(( (ring % 6) + 1 ))
        done
    done
    clear_all
}

# ==============================================================================
# 2. Змейка по общему периметру часов (Периметр 4 разрядов)
# ==============================================================================
anim_snake() {
    printf "${C_GREEN}▶ [2] Змейка по контуру экрана (бежит по периметру дисплея)${C_RESET}\n"
    clear_all
    # Траектория по внешнему краю экрана:
    # A1 -> A2 -> A3 -> A4 -> B4 -> C4 -> D4 -> D3 -> D2 -> D1 -> E1 -> F1
    PATH_SEGS="A1 A2 A3 A4 B4 C4 D4 D3 D2 D1 E1 F1"
    for cycle in $(seq 1 4); do
        ring=1
        for p in $PATH_SEGS; do
            set_seg "$p" 150
            clear_ring
            set_ring "$ring" 0 220 180
            usleep 60000
            set_seg "$p" 0
            ring=$(( (ring % 6) + 1 ))
        done
    done
    clear_all
}

# ==============================================================================
# 3. Живые Глазки / Эмоции Станции (Моргание, взгляд по сторонам, подмигивание)
# ==============================================================================
anim_eyes() {
    printf "${C_GREEN}▶ [3] Живые Глазки (эмоциональный компаньон: моргание, взгляд, подмигивание)${C_RESET}\n"
    clear_all

    # 1. Глазки открыты (разряды 2 и 3)
    show_open_eyes() {
        clear_clock
        # Большие открытые круглые глаза
        for s in A B C D E F; do
            set_seg "${s}2" 140
            set_seg "${s}3" 140
        done
    }

    # Моргание: открыты -> полоска (G2, G3) -> открыты
    blink() {
        clear_clock
        set_seg "G2" 140
        set_seg "G3" 140
        set_seg "MINUS" 140
        usleep 90000
        show_open_eyes
    }

    # Взгляд влево
    look_left() {
        clear_clock
        for s in A F E D; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    }

    # Взгляд вправо
    look_right() {
        clear_clock
        for s in A B C D; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    }

    # Счастливые глазки (дуги ^^)
    happy_eyes() {
        clear_clock
        for s in A B F G; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    }

    # Подмигивание (левый глаз открыт, правый закрыт в тире)
    wink() {
        clear_clock
        for s in A B C D E F; do set_seg "${s}2" 140; done
        set_seg "G3" 140
        # Кольцо подмигивает нежным золотым цветом
        set_ring 3 255 180 0
        set_ring 4 255 180 0
        usleep 300000
        clear_ring
        show_open_eyes
    }

    show_open_eyes
    set_ring_all 0 100 200
    usleep 600000

    # Моргание
    blink
    usleep 500000

    # Взгляд влево
    look_left
    usleep 700000

    # Взгляд вправо
    look_right
    usleep 700000

    # Возврат в центр + двойное моргание
    show_open_eyes
    usleep 300000
    blink
    usleep 150000
    blink
    usleep 400000

    # Подмигивание
    wink
    usleep 600000

    # Счастливые глазки ^^
    happy_eyes
    set_ring_all 0 255 100
    usleep 900000

    clear_all
}

# ==============================================================================
# 4. Эквалайзер 1: Прыгающие столбики (Классический FFT спектр)
# ==============================================================================
anim_equalizer_bars() {
    printf "${C_GREEN}▶ [4] Эквалайзер: Прыгающие столбики спектра (имитация ритмичного бита)${C_RESET}\n"
    clear_all

    set_col() {
        col=$1
        level=$2
        b=130
        clear_seg "D${col}"
        clear_seg "G${col}"
        clear_seg "A${col}"
        if [ "$level" -ge 1 ]; then set_seg "D${col}" "$b"; fi
        if [ "$level" -ge 2 ]; then set_seg "G${col}" "$b"; fi
        if [ "$level" -ge 3 ]; then set_seg "A${col}" "$b"; fi
    }

    # Симуляция 24 кадров динамичного трека
    FRAMES="3 2 1 1   2 3 2 1   1 2 3 2   3 3 1 2   2 1 2 3   3 2 3 1   1 3 2 2   2 3 3 1   3 1 2 2   1 2 3 3   2 3 1 2   3 2 2 1"
    for frame in $FRAMES; do
        l1=$(echo "$frame" | cut -c1)
        l2=$(echo "$frame" | cut -c2)
        l3=$(echo "$frame" | cut -c3)
        l4=$(echo "$frame" | cut -c4)
        set_col 1 "$l1"
        set_col 2 "$l2"
        set_col 3 "$l3"
        set_col 4 "$l4"

        # Вспышка кольца на мощный бас (когда 1-я колонка = 3)
        if [ "$l1" -eq 3 ]; then
            set_ring_all 255 60 0
        else
            set_ring_all 20 5 0
        fi
        usleep 90000
    done
    clear_all
}

# ==============================================================================
# 5. Бобинный магнитофон / Кассетник (Вращение катушек на разрядах 1 и 4)
# ==============================================================================
anim_cassette() {
    printf "${C_GREEN}▶ [5] Кассетник / Бобины (вращение левой и правой катушек, лента по центру)${C_RESET}\n"
    clear_all

    # Лента по центру протянута постоянно
    set_seg "D2" 100
    set_seg "D3" 100
    set_seg "MINUS" 100

    # 4 фазы вращения спиц катушки на разрядах 1 и 4:
    # Фаза 1: A и D (вертикаль)
    # Фаза 2: B и E (диагональ /)
    # Фаза 3: C и F (диагональ \)
    # Фаза 4: G (горизонталь)
    for cycle in $(seq 1 8); do
        # Фаза 1
        clear_seg "G1"; clear_seg "G4"
        set_seg "A1"; set_seg "D1"; set_seg "A4"; set_seg "D4"
        set_ring 1 120 0 180; set_ring 4 120 0 180
        set_ring 2 0 0 0; set_ring 5 0 0 0
        usleep 100000

        # Фаза 2
        clear_seg "A1"; clear_seg "D1"; clear_seg "A4"; clear_seg "D4"
        set_seg "B1"; set_seg "E1"; set_seg "B4"; set_seg "E4"
        set_ring 2 120 0 180; set_ring 5 120 0 180
        set_ring 1 0 0 0; set_ring 4 0 0 0
        usleep 100000

        # Фаза 3
        clear_seg "B1"; clear_seg "E1"; clear_seg "B4"; clear_seg "E4"
        set_seg "C1"; set_seg "F1"; set_seg "C4"; set_seg "F4"
        set_ring 3 120 0 180; set_ring 6 120 0 180
        set_ring 2 0 0 0; set_ring 5 0 0 0
        usleep 100000

        # Фаза 4
        clear_seg "C1"; clear_seg "F1"; clear_seg "C4"; clear_seg "F4"
        set_seg "G1"; set_seg "G4"
        set_ring 1 60 0 100; set_ring 4 60 0 100
        set_ring 3 0 0 0; set_ring 6 0 0 0
        usleep 100000
    done
    clear_all
}

# ==============================================================================
# 6. Переключение трека (Стрелки >> вправо и << влево с эффектом кольца)
# ==============================================================================
anim_track_switch() {
    printf "${C_GREEN}▶ [6] Переключение треков (анимация сдвига шевронов >> и <<)${C_RESET}\n"
    clear_all

    # Следующий трек: шеврон бежит слева направо
    printf "   -> Next Track (>>)\n"
    for step in 1 2 3 4; do
        clear_clock
        draw_char "$step" '>' 150
        clear_ring
        set_ring "$step" 220 0 255
        usleep 90000
    done
    # Финал: двойной шеврон >>
    clear_clock
    draw_char 2 '>' 150
    draw_char 3 '>' 150
    set_ring_all 220 0 255
    usleep 350000

    clear_all
    usleep 200000

    # Предыдущий трек: шеврон бежит справа налево
    printf "   <- Prev Track (<<)\n"
    for step in 4 3 2 1; do
        clear_clock
        draw_char "$step" '<' 150
        clear_ring
        set_ring "$step" 0 200 255
        usleep 90000
    done
    # Финал: двойной шеврон <<
    clear_clock
    draw_char 2 '<' 150
    draw_char 3 '<' 150
    set_ring_all 0 200 255
    usleep 350000

    clear_all
}

# ==============================================================================
# 7. Пульс / Дыхание (Heartbeat pulse из центра наружу)
# ==============================================================================
anim_heartbeat() {
    printf "${C_GREEN}▶ [7] Пульс / Дыхание (расширение из центральной точки + мягкое кольцо)${C_RESET}\n"
    clear_all

    for cycle in $(seq 1 4); do
        # 1. Центр: только MINUS
        clear_clock
        set_seg "MINUS" 150
        set_ring_all 40 0 0
        usleep 80000

        # 2. Средние полоски: G2, G3, MINUS
        set_seg "G2" 150; set_seg "G3" 150
        set_ring_all 120 0 20
        usleep 80000

        # 3. Полные средние цифры 2 и 3
        draw_char 2 '0' 150
        draw_char 3 '0' 150
        set_seg "MINUS" 150
        set_ring_all 255 10 40
        usleep 120000

        # 4. Вспышка на все 4 цифры
        draw_char 1 '0' 120
        draw_char 4 '0' 120
        set_ring_all 255 40 80
        usleep 100000

        # 5. Мягкое угасание
        clear_clock
        draw_char 2 '0' 50
        draw_char 3 '0' 50
        set_ring_all 60 5 15
        usleep 120000

        clear_all
        usleep 300000
    done
    clear_all
}

# ==============================================================================
# 8. Бегущая строка (Marquee ticker: "SENTINEL MUSIC")
# ==============================================================================
anim_marquee() {
    printf "${C_GREEN}▶ [8] Бегущая строка (плавный скролл 'SENTINEL MUSIC')${C_RESET}\n"
    clear_all

    # Текст с отступами для плавного входа и выхода
    TEXT="   SENTINEL MUSIC   "
    LEN=${#TEXT}
    END=$((LEN - 3))

    ring=1
    i=1
    while [ "$i" -le "$END" ]; do
        # Извлекаем 4 символа в текущем окне скролла
        SUB=$(echo "$TEXT" | cut -c "$i"-$((i + 3)))
        draw_word "$SUB" 140

        clear_ring
        set_ring "$ring" 0 180 255
        ring=$(( (ring % 6) + 1 ))

        usleep 160000
        i=$((i + 1))
    done
    clear_all
}

# ==============================================================================
# 9. Матрица / Цифровой дождь (Быстрое падение сегментов сверху вниз)
# ==============================================================================
anim_matrix() {
    printf "${C_GREEN}▶ [9] Матрица / Цифровой дождь (вертикальное падение капель сегментов)${C_RESET}\n"
    clear_all

    for cycle in $(seq 1 12); do
        # Случайный выбор колонки 1..4 через секунды
        col=$(( (cycle % 4) + 1 ))

        # Сверху: A
        set_seg "A${col}" 140
        set_ring "$col" 0 255 60
        usleep 45000

        # Середина: F, B, G
        clear_seg "A${col}"
        set_seg "F${col}" 120; set_seg "B${col}" 120; set_seg "G${col}" 140
        usleep 45000

        # Низ: E, C, D
        clear_seg "F${col}"; clear_seg "B${col}"; clear_seg "G${col}"
        set_seg "E${col}" 100; set_seg "C${col}" 100; set_seg "D${col}" 140
        usleep 45000

        clear_seg "E${col}"; clear_seg "C${col}"; clear_seg "D${col}"
        clear_ring
    done
    clear_all
}

# ==============================================================================
# 10. Кольцо Подсветки: Радужная волна и кометный маяк
# ==============================================================================
anim_ring_colors() {
    printf "${C_GREEN}▶ [10] Кольцо подсветки: Кометный маяк с хвостом и переливы цветов${C_RESET}\n"
    clear_all

    # Выводим надпись RING на экран
    draw_word "RING" 100

    # Вращающийся кометный маяк с затухающим следом (фиолетовый -> ультрамарин)
    for cycle in $(seq 1 4); do
        for head in 1 2 3 4 5 6; do
            tail1=$(( ((head + 4) % 6) + 1 ))
            tail2=$(( ((head + 3) % 6) + 1 ))

            clear_ring
            set_ring "$head"  220 0 255  # Яркая голова кометы
            set_ring "$tail1" 80 0 150   # Средний хвост
            set_ring "$tail2" 20 0 40    # Тусклый хвост
            usleep 80000
        done
    done

    # Плавная смена цветов на всем кольце (Неон -> Бирюза -> Изумруд -> Золото -> Рубин)
    printf "   -> Плавные цветовые акценты кольца\n"
    COLORS="180:0:255 0:180:255 0:255:100 255:180:0 255:20:40"
    for c in $COLORS; do
        r=$(echo "$c" | cut -d: -f1)
        g=$(echo "$c" | cut -d: -f2)
        b=$(echo "$c" | cut -d: -f3)
        set_ring_all "$r" "$g" "$b"
        usleep 350000
    done

    clear_all
}

# ==============================================================================
# Демонстрация ВСЕХ вариантов по очереди
# ==============================================================================
run_all_demo() {
    printf "\n${C_YELLOW}★ ЗАПУСК ПОЛНОГО ДЕМО-ТУРА ВСЕХ 10 АНИМАЦИЙ ★${C_RESET}\n\n"
    anim_spinner
    usleep 400000
    anim_snake
    usleep 400000
    anim_eyes
    usleep 400000
    anim_equalizer_bars
    usleep 400000
    anim_cassette
    usleep 400000
    anim_track_switch
    usleep 400000
    anim_heartbeat
    usleep 400000
    anim_marquee
    usleep 400000
    anim_matrix
    usleep 400000
    anim_ring_colors
    usleep 400000
    printf "\n${C_GREEN}✓ Полное демо завершено!${C_RESET}\n\n"
}

# ==============================================================================
# Восстановление системы при выходе
# ==============================================================================
cleanup() {
    trap - INT TERM EXIT
    clear_all
    printf "\n${C_YELLOW}Восстановление службы Sentinel Station...${C_RESET}\n"
    /etc/init.d/S99sentinel-station start >/dev/null 2>&1
    printf "${C_GREEN}Служба перезапущена, штатный экран восстановлен.${C_RESET}\n"
    exit 0
}

trap cleanup INT TERM EXIT

# ==============================================================================
# Главное интерактивное меню
# ==============================================================================
main() {
    # Если передан аргумент (например ./test_animations.sh 3 или ./test_animations.sh A), запускаем сразу
    if [ -n "$1" ]; then
        /etc/init.d/S99sentinel-station stop >/dev/null 2>&1
        clear_all
        case "$1" in
            1) anim_spinner ;;
            2) anim_snake ;;
            3) anim_eyes ;;
            4) anim_equalizer_bars ;;
            5) anim_cassette ;;
            6) anim_track_switch ;;
            7) anim_heartbeat ;;
            8) anim_marquee ;;
            9) anim_matrix ;;
            10) anim_ring_colors ;;
            A|a|all|ALL) run_all_demo ;;
            *) printf "${C_YELLOW}Неверный аргумент: $1. Используйте 1..10 или A.${C_RESET}\n" ;;
        esac
        cleanup
    fi

    printf "${C_YELLOW}Временная остановка фоновой службы Sentinel для прямого доступа к LED...${C_RESET}\n"
    /etc/init.d/S99sentinel-station stop >/dev/null 2>&1
    clear_all

    while true; do
        printf "\n${C_CYAN}====================================================================${C_RESET}\n"
        printf "${C_MAGENTA}   ГАЛЕРЕЯ АНИМАЦИЙ СТАНЦИИ МИНИ 2 (Экран + RGB-Кольцо)${C_RESET}\n"
        printf "${C_CYAN}====================================================================${C_RESET}\n"
        printf " ${C_YELLOW}[1]${C_RESET}  Спиннер / Радар (круговое вращение по разрядам + кольцо)\n"
        printf " ${C_YELLOW}[2]${C_RESET}  Змейка по контуру экрана (ползет по периметру 4 разрядов)\n"
        printf " ${C_YELLOW}[3]${C_RESET}  Живые Глазки (эмоциональный компаньон: моргание, взгляд, подмигивание)\n"
        printf " ${C_YELLOW}[4]${C_RESET}  Эквалайзер спектра (динамичные прыгающие столбики под бас)\n"
        printf " ${C_YELLOW}[5]${C_RESET}  Кассетник / Бобины (вращение катушек на боковых разрядах)\n"
        printf " ${C_YELLOW}[6]${C_RESET}  Переключение трека (анимированные стрелки >> вправо и << влево)\n"
        printf " ${C_YELLOW}[7]${C_RESET}  Пульс / Дыхание сердца (расширение из центра + мягкий свет)\n"
        printf " ${C_YELLOW}[8]${C_RESET}  Бегущая строка (плавный скролл 'SENTINEL MUSIC')\n"
        printf " ${C_YELLOW}[9]${C_RESET}  Матрица / Цифровой дождь (падающие капли сегментов)\n"
        printf " ${C_YELLOW}[10]${C_RESET} Кольцо подсветки (кометный маяк с хвостом + неоновые переливы)\n"
        printf " --------------------------------------------------------------------\n"
        printf " ${C_GREEN}[A]${C_RESET}  Показать ВСЕ 10 анимаций по очереди (Полный демо-тур)\n"
        printf " ${C_RED}[0]${C_RESET}  Выход (возврат к штатным часам Sentinel)\n"
        printf "${C_CYAN}====================================================================${C_RESET}\n"
        printf "Выберите номер [0-10 или A]: "

        if ! read -r choice; then
            break
        fi
        case "$choice" in
            1) anim_spinner ;;
            2) anim_snake ;;
            3) anim_eyes ;;
            4) anim_equalizer_bars ;;
            5) anim_cassette ;;
            6) anim_track_switch ;;
            7) anim_heartbeat ;;
            8) anim_marquee ;;
            9) anim_matrix ;;
            10) anim_ring_colors ;;
            A|a|all|ALL) run_all_demo ;;
            0|q|Q|exit) break ;;
            *) printf "${C_YELLOW}Неверный выбор, введите число от 1 до 10, A или 0.${C_RESET}\n" ;;
        esac
    done
}

main "$@"
