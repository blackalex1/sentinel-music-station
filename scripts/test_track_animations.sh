#!/bin/sh
# ==============================================================================
# Sentinel Station — Специальная витрина анимаций переключения треков (Next/Prev)
# ==============================================================================
# Позволяет наглядно протестировать различные эффекты переключения треков:
# - Шторка / Сканер (плавный сдвиг вертикального луча)
# - Бегущие шевроны (классические и варп-стрелки >> и <<)
# - Перемотка бобин / DJ винил скретч
# - Каскадный барабан (вертикальный скролл)
# - Лазерный луч по центру (Cylon beam)
# - Сверхзвуковая волна из центра
# - Орбитальная комета (экран -> кольцо)
# - Текстовые бейджи "NEXT" / "PREV"
# ==============================================================================

BRIGHTNESS=130

# Цвета ANSI для меню
C_CYAN="\033[1;36m"
C_GREEN="\033[1;32m"
C_YELLOW="\033[1;33m"
C_MAGENTA="\033[1;35m"
C_RESET="\033[0m"

set_seg() {
    echo "${2:-$BRIGHTNESS}" > "/sys/class/leds/mini2:white:clock$1/brightness" 2>/dev/null
}

clear_seg() {
    echo 0 > "/sys/class/leds/mini2:white:clock$1/brightness" 2>/dev/null
}

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
    _d=$1
    _ch=$2
    _b=${3:-$BRIGHTNESS}
    for _s in A B C D E F G; do
        clear_seg "${_s}${_d}"
    done
    case "$_ch" in
        '>') for _s in A B G C D; do set_seg "${_s}${_d}" "$_b"; done ;;
        '<') for _s in A F G E D; do set_seg "${_s}${_d}" "$_b"; done ;;
        'N'|'n') for _s in E G C; do set_seg "${_s}${_d}" "$_b"; done ;;
        'E'|'e') for _s in A F G E D; do set_seg "${_s}${_d}" "$_b"; done ;;
        'X'|'x') for _s in B C E F G; do set_seg "${_s}${_d}" "$_b"; done ;;
        'T'|'t') for _s in F G E D; do set_seg "${_s}${_d}" "$_b"; done ;;
        'P'|'p') for _s in A B G E F; do set_seg "${_s}${_d}" "$_b"; done ;;
        'R'|'r') for _s in E G; do set_seg "${_s}${_d}" "$_b"; done ;;
        'V'|'v') for _s in E D C; do set_seg "${_s}${_d}" "$_b"; done ;;
        '0') for _s in A B C D E F; do set_seg "${_s}${_d}" "$_b"; done ;;
        '-') set_seg "G${_d}" "$_b" ;;
        '|') for _s in E F; do set_seg "${_s}${_d}" "$_b"; done ;;
    esac
}

draw_word() {
    w="$1"
    b=${2:-$BRIGHTNESS}
    draw_char 1 "$(echo "$w" | cut -c1)" "$b"
    draw_char 2 "$(echo "$w" | cut -c2)" "$b"
    draw_char 3 "$(echo "$w" | cut -c3)" "$b"
    draw_char 4 "$(echo "$w" | cut -c4)" "$b"
}

# ==============================================================================
# 1. Шторка сканера / Плавный вертикальный сдвиг (8 позиций)
# ==============================================================================
# Самая плавная анимация: вертикальная линия движется по 8 вертикальным ребрам
# (F1/E1 -> B1/C1 -> F2/E2 -> B2/C2 -> F3/E3 -> B3/C3 -> F4/E4 -> B4/C4)
anim_scanner_curtain() {
    printf "${C_GREEN}▶ [1] Шторка / Сканер (плавный сдвиг вертикального луча по 8 ребрам)${C_RESET}\n"
    clear_all

    set_slice() {
        pos=$1
        b=${2:-$BRIGHTNESS}
        case "$pos" in
            1) set_seg "F1" "$b"; set_seg "E1" "$b" ;;
            2) set_seg "B1" "$b"; set_seg "C1" "$b" ;;
            3) set_seg "F2" "$b"; set_seg "E2" "$b" ;;
            4) set_seg "B2" "$b"; set_seg "C2" "$b" ;;
            5) set_seg "F3" "$b"; set_seg "E3" "$b" ;;
            6) set_seg "B3" "$b"; set_seg "C3" "$b" ;;
            7) set_seg "F4" "$b"; set_seg "E4" "$b" ;;
            8) set_seg "B4" "$b"; set_seg "C4" "$b" ;;
        esac
    }

    clear_slice() {
        pos=$1
        case "$pos" in
            1) clear_seg "F1"; clear_seg "E1" ;;
            2) clear_seg "B1"; clear_seg "C1" ;;
            3) clear_seg "F2"; clear_seg "E2" ;;
            4) clear_seg "B2"; clear_seg "C2" ;;
            5) clear_seg "F3"; clear_seg "E3" ;;
            6) clear_seg "B3"; clear_seg "C3" ;;
            7) clear_seg "F4"; clear_seg "E4" ;;
            8) clear_seg "B4"; clear_seg "C4" ;;
        esac
    }

    printf "   -> Next Track (слева направо >>>)\n"
    ring=1
    for i in 1 2 3 4 5 6 7 8; do
        clear_clock
        set_slice "$i" 160
        if [ "$i" -gt 1 ]; then set_slice "$((i - 1))" 60; fi
        clear_ring
        set_ring "$ring" 220 0 255
        usleep 45000
        ring=$(( (ring % 6) + 1 ))
    done
    clear_all
    usleep 300000

    printf "   <- Prev Track (справа налево <<<)\n"
    ring=6
    for i in 8 7 6 5 4 3 2 1; do
        clear_clock
        set_slice "$i" 160
        if [ "$i" -lt 8 ]; then set_slice "$((i + 1))" 60; fi
        clear_ring
        set_ring "$ring" 0 200 255
        usleep 45000
        ring=$(( (ring + 4) % 6 + 1 ))
    done
    clear_all
}

# ==============================================================================
# 2. Бегущий шеврон с ускорением (Стрелка > и < + двойной шеврон)
# ==============================================================================
anim_chevron_slide() {
    printf "${C_GREEN}▶ [2] Бегущий шеврон (одиночная стрелка ускоряется и превращается в >>)${C_RESET}\n"
    clear_all

    printf "   -> Next Track (>>)\n"
    delays="110000 85000 65000 45000"
    pos=1
    for dl in $delays; do
        clear_clock
        draw_char "$pos" '>' 150
        clear_ring
        set_ring "$pos" 200 0 255
        usleep "$dl"
        pos=$((pos + 1))
    done
    # Финал: фиксация двойного шеврона >>
    clear_clock
    draw_char 2 '>' 150
    draw_char 3 '>' 150
    set_ring_all 200 0 255
    usleep 280000
    clear_all
    usleep 250000

    printf "   <- Prev Track (<<)\n"
    pos=4
    for dl in $delays; do
        clear_clock
        draw_char "$pos" '<' 150
        clear_ring
        set_ring "$pos" 0 200 255
        usleep "$dl"
        pos=$((pos - 1))
    done
    clear_clock
    draw_char 2 '<' 150
    draw_char 3 '<' 150
    set_ring_all 0 200 255
    usleep 280000
    clear_all
}

# ==============================================================================
# 3. Лазерный луч по центру (Cylon / Knight Rider Beam)
# ==============================================================================
# Горизонтальный лазерный импульс проносится по средней линии (G1->G2->MINUS->G3->G4)
anim_cylon_beam() {
    printf "${C_GREEN}▶ [3] Лазерный луч (горизонтальный световой импульс по средней линии)${C_RESET}\n"
    clear_all

    printf "   -> Next Track (лазер летит вправо)\n"
    # Быстрый двойной прострел вправо
    for pass in 1 2; do
        for p in "G1" "G2" "MINUS" "G3" "G4"; do
            clear_clock
            set_seg "$p" 160
            usleep 35000
        done
    done
    # Вспышка на правой цифре и кольце
    draw_char 4 '>' 150
    set_ring 3 255 120 0; set_ring 4 255 120 0
    usleep 220000
    clear_all
    usleep 250000

    printf "   <- Prev Track (лазер летит влево)\n"
    for pass in 1 2; do
        for p in "G4" "G3" "MINUS" "G2" "G1"; do
            clear_clock
            set_seg "$p" 160
            usleep 35000
        done
    done
    draw_char 1 '<' 150
    set_ring 6 0 220 255; set_ring 1 0 220 255
    usleep 220000
    clear_all
}

# ==============================================================================
# 4. Перемотка бобин / Кассетный ревайнд (Быстрое кручение катушек 1 и 4)
# ==============================================================================
anim_tape_rewind() {
    printf "${C_GREEN}▶ [4] Перемотка бобин (быстрое вращение катушек на 1 и 4 цифрах)${C_RESET}\n"
    clear_all

    printf "   -> Fast Forward (перемотка вперед, вращение по часовой)\n"
    set_seg "D2" 80; set_seg "D3" 80; set_seg "MINUS" 80
    ring=1
    for cycle in $(seq 1 4); do
        for s in A B C D E F; do
            set_seg "${s}1" 140; set_seg "${s}4" 140
            clear_ring
            set_ring "$ring" 180 0 255
            usleep 30000
            clear_seg "${s}1"; clear_seg "${s}4"
            ring=$(( (ring % 6) + 1 ))
        done
    done
    clear_all
    usleep 250000

    printf "   <- Rewind (перемотка назад, вращение против часовой)\n"
    set_seg "D2" 80; set_seg "D3" 80; set_seg "MINUS" 80
    ring=6
    for cycle in $(seq 1 4); do
        for s in F E D C B A; do
            set_seg "${s}1" 140; set_seg "${s}4" 140
            clear_ring
            set_ring "$ring" 0 200 255
            usleep 30000
            clear_seg "${s}1"; clear_seg "${s}4"
            ring=$(( (ring + 4) % 6 + 1 ))
        done
    done
    clear_all
}

# ==============================================================================
# 5. Каскадный барабан / Вертикальный ролл (Slot Machine Roll)
# ==============================================================================
# Горизонтальные сегменты (D -> G -> A) перекатываются вертикально, как барабаны
anim_slot_roll() {
    printf "${C_GREEN}▶ [5] Каскадный барабан (вертикальный перекат D -> G -> A)${C_RESET}\n"
    clear_all

    printf "   -> Next Track (перекат снизу вверх D -> G -> A)\n"
    for col in 1 2 3 4; do
        clear_clock
        set_seg "D${col}" 140
        usleep 40000
        clear_seg "D${col}"
        set_seg "G${col}" 140
        usleep 40000
        clear_seg "G${col}"
        set_seg "A${col}" 140
        clear_ring
        set_ring "$col" 220 0 255
        usleep 40000
    done
    # Фиксация стрелки вправо
    clear_clock
    draw_char 3 '>' 150
    draw_char 4 '>' 150
    set_ring_all 220 0 255
    usleep 250000
    clear_all
    usleep 250000

    printf "   <- Prev Track (перекат сверху вниз A -> G -> D)\n"
    for col in 4 3 2 1; do
        clear_clock
        set_seg "A${col}" 140
        usleep 40000
        clear_seg "A${col}"
        set_seg "G${col}" 140
        usleep 40000
        clear_seg "G${col}"
        set_seg "D${col}" 140
        clear_ring
        set_ring "$col" 0 200 255
        usleep 40000
    done
    clear_clock
    draw_char 1 '<' 150
    draw_char 2 '<' 150
    set_ring_all 0 200 255
    usleep 250000
    clear_all
}

# ==============================================================================
# 6. Скретч винила (DJ Vinyl Scratch & Drop)
# ==============================================================================
anim_vinyl_scratch() {
    printf "${C_GREEN}▶ [6] Скретч винила (DJ скретч вперед-назад перед дропом трека)${C_RESET}\n"
    clear_all

    printf "   -> Next Track (скретч 'вика-вика' + рывок вперед)\n"
    # Скретч туда-сюда на цифрах 2 и 3
    for s in A B G; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    set_ring_all 255 0 100
    usleep 70000
    clear_clock
    for s in G E D; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    set_ring_all 0 200 255
    usleep 70000
    clear_clock
    for s in A B G; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    set_ring_all 255 0 100
    usleep 70000
    # Дроп трека: быстрый круг и появление >>
    clear_clock
    draw_char 2 '>' 150
    draw_char 3 '>' 150
    for r in 1 2 3 4 5 6; do
        clear_ring
        set_ring "$r" 255 200 0
        usleep 25000
    done
    set_ring_all 255 200 0
    usleep 250000
    clear_all
    usleep 250000

    printf "   <- Prev Track (обратный скретч + рывок назад)\n"
    for s in A F G; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    set_ring_all 0 200 255
    usleep 70000
    clear_clock
    for s in G C D; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    set_ring_all 255 0 100
    usleep 70000
    clear_clock
    for s in A F G; do set_seg "${s}2" 140; set_seg "${s}3" 140; done
    set_ring_all 0 200 255
    usleep 70000
    clear_clock
    draw_char 2 '<' 150
    draw_char 3 '<' 150
    for r in 6 5 4 3 2 1; do
        clear_ring
        set_ring "$r" 0 255 200
        usleep 25000
    done
    set_ring_all 0 255 200
    usleep 250000
    clear_all
}

# ==============================================================================
# 7. Звуковая ударная волна (Sonic Shockwave из центра)
# ==============================================================================
anim_sonic_shockwave() {
    printf "${C_GREEN}▶ [7] Звуковая волна (ударная вспышка из центра MINUS на весь экран)${C_RESET}\n"
    clear_all

    printf "   -> Next Track (волна взрывается вправо)\n"
    # 1. Зарядка в центре
    set_seg "MINUS" 150
    set_ring_all 50 0 80
    usleep 70000
    # 2. Расширение на центр
    set_seg "G2" 140; set_seg "G3" 140
    set_ring_all 150 0 200
    usleep 60000
    # 3. Вспышка на правые цифры 3 и 4
    clear_clock
    draw_char 3 '>' 150
    draw_char 4 '>' 150
    set_ring_all 255 50 255
    usleep 250000
    clear_all
    usleep 250000

    printf "   <- Prev Track (волна взрывается влево)\n"
    set_seg "MINUS" 150
    set_ring_all 0 50 80
    usleep 70000
    set_seg "G2" 140; set_seg "G3" 140
    set_ring_all 0 150 200
    usleep 60000
    clear_clock
    draw_char 1 '<' 150
    draw_char 2 '<' 150
    set_ring_all 0 220 255
    usleep 250000
    clear_all
}

# ==============================================================================
# 8. Орбитальная комета (Облетает экран и перелетает на кольцо)
# ==============================================================================
anim_orbital_comet() {
    printf "${C_GREEN}▶ [8] Орбитальная комета (траектория: по верху экрана -> круг на кольце)${C_RESET}\n"
    clear_all

    printf "   -> Next Track (полет по часовой стрелке)\n"
    # Полет по верхним сегментам экрана слева направо: A1 -> A2 -> A3 -> A4 -> DEGREE
    for p in "A1" "A2" "A3" "A4" "DEGREE"; do
        clear_clock
        set_seg "$p" 160
        usleep 45000
    done
    clear_clock
    # Переход на кольцо: оборот вокруг кнопки
    for r in 1 2 3 4 5 6; do
        clear_ring
        set_ring "$r" 0 255 180
        usleep 40000
    done
    # Финальная стрелка
    draw_char 3 '>' 140
    draw_char 4 '>' 140
    set_ring_all 0 255 180
    usleep 250000
    clear_all
    usleep 250000

    printf "   <- Prev Track (полет против часовой стрелки)\n"
    for p in "DEGREE" "A4" "A3" "A2" "A1"; do
        clear_clock
        set_seg "$p" 160
        usleep 45000
    done
    clear_clock
    for r in 6 5 4 3 2 1; do
        clear_ring
        set_ring "$r" 255 180 0
        usleep 40000
    done
    draw_char 1 '<' 140
    draw_char 2 '<' 140
    set_ring_all 255 180 0
    usleep 250000
    clear_all
}

# ==============================================================================
# 9. Варп-поток шевронов (Hyperspace Warp Chevrons: >> >> >>)
# ==============================================================================
anim_warp_stream() {
    printf "${C_GREEN}▶ [9] Варп-поток шевронов (серия стремительно летящих стрелок >> >> >>)${C_RESET}\n"
    clear_all

    printf "   -> Next Track (варп-прыжок вперед)\n"
    for wave in 1 2 3; do
        for step in 1 2 3 4; do
            clear_clock
            draw_char "$step" '>' 150
            clear_ring
            set_ring "$step" 220 0 255
            usleep 35000
        done
    done
    clear_clock
    draw_char 2 '>' 150
    draw_char 3 '>' 150
    set_ring_all 220 0 255
    usleep 250000
    clear_all
    usleep 250000

    printf "   <- Prev Track (варп-прыжок назад)\n"
    for wave in 1 2 3; do
        for step in 4 3 2 1; do
            clear_clock
            draw_char "$step" '<' 150
            clear_ring
            set_ring "$step" 0 200 255
            usleep 35000
        done
    done
    clear_clock
    draw_char 2 '<' 150
    draw_char 3 '<' 150
    set_ring_all 0 200 255
    usleep 250000
    clear_all
}

# ==============================================================================
# 10. Текстовый бейдж + шеврон ("NEXT >>" и "PREV <<")
# ==============================================================================
anim_badge_text() {
    printf "${C_GREEN}▶ [10] Текстовый бейдж (мгновенная вспышка слова NEXT/PREV + стрелки)${C_RESET}\n"
    clear_all

    printf "   -> Next Track ('NEXT' -> ' >> ')\n"
    # Вспышка слова NEXT
    draw_word "NEXT" 140
    set_ring_all 200 0 255
    usleep 220000
    # Плавное превращение в >>
    clear_clock
    draw_char 2 '>' 150
    draw_char 3 '>' 150
    usleep 250000
    clear_all
    usleep 250000

    printf "   <- Prev Track ('PREV' -> ' << ')\n"
    draw_word "PREV" 140
    set_ring_all 0 200 255
    usleep 220000
    clear_clock
    draw_char 2 '<' 150
    draw_char 3 '<' 150
    usleep 250000
    clear_all
}

# ==============================================================================
# Полный демо-тур всех вариантов переключения
# ==============================================================================
run_all_demo() {
    printf "\n${C_YELLOW}★ ПОЛНЫЙ ДЕМО-ТУР ВСЕХ 10 ВАРИАНТОВ ПЕРЕКЛЮЧЕНИЯ ТРЕКОВ ★${C_RESET}\n\n"
    anim_scanner_curtain
    usleep 350000
    anim_chevron_slide
    usleep 350000
    anim_cylon_beam
    usleep 350000
    anim_tape_rewind
    usleep 350000
    anim_slot_roll
    usleep 350000
    anim_vinyl_scratch
    usleep 350000
    anim_sonic_shockwave
    usleep 350000
    anim_orbital_comet
    usleep 350000
    anim_warp_stream
    usleep 350000
    anim_badge_text
    usleep 350000
    printf "\n${C_GREEN}✓ Демо-тур вариантов переключения треков завершен!${C_RESET}\n\n"
}

cleanup() {
    trap - INT TERM EXIT
    clear_all
    printf "\n${C_YELLOW}Восстановление службы Sentinel Station...${C_RESET}\n"
    /etc/init.d/S99sentinel-station start >/dev/null 2>&1
    printf "${C_GREEN}Служба перезапущена, штатный экран восстановлен.${C_RESET}\n"
    exit 0
}

trap cleanup INT TERM EXIT

main() {
    if [ -n "$1" ]; then
        /etc/init.d/S99sentinel-station stop >/dev/null 2>&1
        clear_all
        case "$1" in
            1) anim_scanner_curtain ;;
            2) anim_chevron_slide ;;
            3) anim_cylon_beam ;;
            4) anim_tape_rewind ;;
            5) anim_slot_roll ;;
            6) anim_vinyl_scratch ;;
            7) anim_sonic_shockwave ;;
            8) anim_orbital_comet ;;
            9) anim_warp_stream ;;
            10) anim_badge_text ;;
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
        printf "${C_MAGENTA}   ГАЛЕРЕЯ АНИМАЦИЙ ПЕРЕКЛЮЧЕНИЯ ТРЕКОВ (Next & Prev)${C_RESET}\n"
        printf "${C_CYAN}====================================================================${C_RESET}\n"
        printf " ${C_YELLOW}[1]${C_RESET}  Шторка / Сканер (плавный сдвиг вертикального луча по 8 ребрам)\n"
        printf " ${C_YELLOW}[2]${C_RESET}  Бегущий шеврон (одиночная стрелка ускоряется и переходит в >>)\n"
        printf " ${C_YELLOW}[3]${C_RESET}  Лазерный луч Cylon (горизонтальный световой импульс по средней линии)\n"
        printf " ${C_YELLOW}[4]${C_RESET}  Перемотка бобин (быстрое кручение катушек на цифрах 1 и 4)\n"
        printf " ${C_YELLOW}[5]${C_RESET}  Каскадный барабан (вертикальный перекат D -> G -> A)\n"
        printf " ${C_YELLOW}[6]${C_RESET}  Скретч винила (DJ скретч перед дропом следующего трека)\n"
        printf " ${C_YELLOW}[7]${C_RESET}  Звуковая ударная волна (взрыв из центра MINUS наружу)\n"
        printf " ${C_YELLOW}[8]${C_RESET}  Орбитальная комета (пролет по верху экрана -> круг на кольце)\n"
        printf " ${C_YELLOW}[9]${C_RESET}  Варп-поток (серия стремительно летящих шевронов >> >> >>)\n"
        printf " ${C_YELLOW}[10]${C_RESET} Текстовый бейдж (вспышка слова 'NEXT'/'PREV' + стрелки)\n"
        printf " --------------------------------------------------------------------\n"
        printf " ${C_GREEN}[A]${C_RESET}  Показать ВСЕ 10 вариантов переключения треков по очереди\n"
        printf " ${C_RED}[0]${C_RESET}  Выход (возврат к штатным часам Sentinel)\n"
        printf "${C_CYAN}====================================================================${C_RESET}\n"
        printf "Выберите номер [0-10 или A]: "

        if ! read -r choice; then
            break
        fi
        case "$choice" in
            1) anim_scanner_curtain ;;
            2) anim_chevron_slide ;;
            3) anim_cylon_beam ;;
            4) anim_tape_rewind ;;
            5) anim_slot_roll ;;
            6) anim_vinyl_scratch ;;
            7) anim_sonic_shockwave ;;
            8) anim_orbital_comet ;;
            9) anim_warp_stream ;;
            10) anim_badge_text ;;
            A|a|all|ALL) run_all_demo ;;
            0|q|Q|exit) break ;;
            *) printf "${C_YELLOW}Неверный выбор, введите число от 1 до 10, A или 0.${C_RESET}\n" ;;
        esac
    done
}

main "$@"
