#include "tui_view.h"

#include <curses.h>
#include <chrono>
#include <clocale>
#include <iomanip>
#include <sstream>
#include <string>
#include <vector>
#include <thread> // Added this line

namespace {

// Define color pairs
enum ColorPairs {
    NORMAL_TEXT = 1,
    SHADOW_TEXT,
    BANNER_TEXT,
    SELECTED_ITEM
};

void init_colors() {
    if (has_colors()) {
        start_color();
        use_default_colors(); // Use terminal's default background
        init_pair(NORMAL_TEXT, COLOR_WHITE, -1); // White foreground, default background
        init_pair(SHADOW_TEXT, COLOR_BLACK, -1); // Black foreground, default background (for shadow effect)
        init_pair(BANNER_TEXT, COLOR_CYAN, -1);  // Cyan foreground, default background
        init_pair(SELECTED_ITEM, COLOR_YELLOW, -1); // Yellow foreground, default background
    }
}

std::string format_date(const std::chrono::system_clock::time_point &tp) {
    auto tt = std::chrono::system_clock::to_time_t(tp);
    std::tm tm{};
#if defined(_WIN32)
    localtime_s(&tm, &tt);
#else
    localtime_r(&tt, &tm);
#endif
    std::ostringstream oss;
    oss << std::put_time(&tm, "%Y-%m-%d %a %H:%M");
    return oss.str();
}

} // namespace

void run_tui(const std::vector<IcsEvent> &events) {
    // 让 ncurses 按当前终端 locale 处理 UTF-8
    setlocale(LC_ALL, "");

    initscr();
    init_colors(); // Initialize colors
    cbreak();
    noecho();
    keypad(stdscr, TRUE);
    curs_set(0);

    int height, width;
    getmaxyx(stdscr, height, width);

    // 如果没有任何事件，给出提示信息
    if (events.empty()) {
        clear();
        attron(COLOR_PAIR(BANNER_TEXT));
        mvprintw(0, 0, "NBTCA 未来一个月活动");
        attroff(COLOR_PAIR(BANNER_TEXT));
        mvprintw(2, 0, "未来一个月内暂无活动。");
        mvprintw(4, 0, "按任意键退出...");
        refresh();
        getch();
        endwin();
        return;
    }

    int top = 0;
    int selected = 0;

    while (true) {
        clear();

        // ASCII art banner for "CALENDAR"
        std::string calendar_banner[] = {
            " ██████╗ █████╗ ██╗     ███████╗███╗   ██╗██████╗  █████╗ ██████╗ ",
            "██╔════╝██╔══██╗██║     ██╔════╝████╗  ██║██╔══██╗██╔══██╗██╔══██╗",
            "██║     ███████║██║     █████╗  ██╔██╗ ██║██║  ██║███████║██████╔╝",
            "██║     ██╔══██║██║     ██╔══╝  ██║╚██╗██║██║  ██║██╔══██║██╔══██╗",
            "╚██████╗██║  ██║███████╗███████╗██║ ╚████║██████╔╝██║  ██║██║  ██║",
            " ╚═════╝╚═╝  ╚═╝╚══════╝╚══════╝╚═╝  ╚═══╝╚═════╝ ╚═╝  ╚═╝╚╝  ╚═╝"
        };

        int banner_height = sizeof(calendar_banner) / sizeof(calendar_banner[0]);
        int banner_width = calendar_banner[0].length();

        int start_col_banner = (width - banner_width) / 2;
        if (start_col_banner < 0) start_col_banner = 0;

        // Draw shadow
        attron(COLOR_PAIR(SHADOW_TEXT));
        for (int i = 0; i < banner_height; ++i) {
            mvprintw(i + 1, start_col_banner + 1, "%s", calendar_banner[i].c_str());
        }
        attroff(COLOR_PAIR(SHADOW_TEXT));

        // Draw main banner
        attron(COLOR_PAIR(BANNER_TEXT));
        for (int i = 0; i < banner_height; ++i) {
            mvprintw(i, start_col_banner, "%s", calendar_banner[i].c_str());
        }
        attroff(COLOR_PAIR(BANNER_TEXT));

        attron(COLOR_PAIR(NORMAL_TEXT));
        std::string instruction_msg = "(q 退出, ↑↓ 滚动)";
        mvprintw(banner_height + 1, (width - instruction_msg.length()) / 2, "%s", instruction_msg.c_str());
        attroff(COLOR_PAIR(NORMAL_TEXT));

        int start_event_row = banner_height + 3; // Start events below the banner and instruction
        int visibleLines = height - start_event_row - 1; // Adjust visible lines

        if (selected < top) {
            top = selected;
        } else if (selected >= top + visibleLines) {
            top = selected - visibleLines + 1;
        }

        for (int i = 0; i < visibleLines; ++i) {
            int idx = top + i;
            if (idx >= static_cast<int>(events.size())) break;

            const auto &ev = events[idx];
            std::string line = format_date(ev.start) + "  " + ev.summary;
            if (!ev.location.empty()) {
                line += " @" + ev.location;
            }

            if ((int)line.size() > width - 1) {
                line.resize(width - 1);
            }

            if (idx == selected) {
                attron(A_REVERSE | COLOR_PAIR(SELECTED_ITEM));
                mvprintw(start_event_row + i, 0, "%s", line.c_str());
                attroff(A_REVERSE | COLOR_PAIR(SELECTED_ITEM));
            } else {
                attron(COLOR_PAIR(NORMAL_TEXT));
                mvprintw(start_event_row + i, 0, "%s", line.c_str());
                attroff(COLOR_PAIR(NORMAL_TEXT));
            }
            // Add a separator after each event
            if (i < visibleLines - 1 && idx + 1 < static_cast<int>(events.size())) {
                attron(COLOR_PAIR(SHADOW_TEXT));
                mvprintw(start_event_row + i + 1, 0, "%s", std::string(width, '-').c_str()); // Dynamic separator
                attroff(COLOR_PAIR(SHADOW_TEXT));
            }
        }

        refresh();

        int ch = getch();
        if (ch == 'q' || ch == 'Q') {
            break;
        } else if (ch == KEY_UP || ch == 'k') {
            if (selected > 0) selected--;
        } else if (ch == KEY_DOWN || ch == 'j') {
            if (selected + 1 < (int)events.size()) selected++;
        }
    }

    endwin();
}

int run_portal_tui() {
    setlocale(LC_ALL, "");

    initscr();
    init_colors(); // Initialize colors
    cbreak();
    noecho();
    keypad(stdscr, TRUE);
    curs_set(0);

    int height, width;
    getmaxyx(stdscr, height, width);

    std::vector<std::string> menu_items = {"Calendar", "Exit"};
    int selected = 0;
    int choice = -1;

    while (choice == -1) {
        clear();

        // ASCII art banner for "NBTCA Tools"
        std::string banner_art[] = {
            "███╗   ██╗██████╗ ████████╗ ██████╗ █████╗     ████████╗ ██████╗  ██████╗ ██╗     ███████╗",
            "████╗  ██║██╔══██╗╚══██╔══╝██╔════╝██╔══██╗    ╚══██╔══╝██╔═══██╗██╔═══██╗██║     ██╔════╝",
            "██╔██╗ ██║██████╔╝   ██║   ██║     ███████║       ██║   ██║   ██║██║   ██║██║     ███████╗",
            "██║╚██╗██║██╔══██╗   ██║   ██║     ██╔══██║       ██║   ██║   ██║██║   ██║██║     ╚════██║",
            "██║ ╚████║██████╔╝   ██║   ╚██████╗██║  ██║       ██║   ╚██████╔╝╚██████╔╝███████╗███████║",
            "╚═╝  ╚═══╝╚═════╝    ╚═╝    ╚═════╝╚═╝  ╚═╝       ╚═╝    ╚═════╝  ╚═════╝ ╚══════╝╚══════╝"
        };

        int banner_height = sizeof(banner_art) / sizeof(banner_art[0]);
        int banner_width = banner_art[0].length();

        int start_col_banner = (width - banner_width) / 2;
        if (start_col_banner < 0) start_col_banner = 0;

        // Draw shadow
        attron(COLOR_PAIR(SHADOW_TEXT));
        for (int i = 0; i < banner_height; ++i) {
            mvprintw(i + 1, start_col_banner + 1, "%s", banner_art[i].c_str());
        }
        attroff(COLOR_PAIR(SHADOW_TEXT));

        // Draw main banner
        attron(COLOR_PAIR(BANNER_TEXT));
        for (int i = 0; i < banner_height; ++i) {
            mvprintw(i, start_col_banner, "%s", banner_art[i].c_str());
        }
        attroff(COLOR_PAIR(BANNER_TEXT));

        attron(COLOR_PAIR(NORMAL_TEXT));
        // Feature Portal message below the banner
        std::string portal_msg = "Feature Portal (q or Enter to select)";
        mvprintw(banner_height + 1, (width - portal_msg.length()) / 2, "%s", portal_msg.c_str());
        attroff(COLOR_PAIR(NORMAL_TEXT));

        for (size_t i = 0; i < menu_items.size(); ++i) {
            if ((int)i == selected) {
                attron(A_REVERSE | COLOR_PAIR(SELECTED_ITEM));
            } else {
                attron(COLOR_PAIR(NORMAL_TEXT));
            }
            // Adjust vertical position based on banner height and portal message
            mvprintw(banner_height + 3 + i, 2, "%s", menu_items[i].c_str());
            attroff(A_REVERSE | COLOR_PAIR(SELECTED_ITEM));
            attroff(COLOR_PAIR(NORMAL_TEXT));
        }

        refresh();

        int ch = getch();
        switch (ch) {
            case KEY_UP:
            case 'k':
                if (selected > 0) {
                    selected--;
                }
                break;
            case KEY_DOWN:
            case 'j':
                if (selected < (int)menu_items.size() - 1) {
                    selected++;
                }
                break;
            case 'q':
            case 'Q':
                choice = 1; // Corresponds to "Exit"
                break;
            case 10: // Enter key
                choice = selected;
                break;
        }
    }

    endwin();
    return choice;
}

// 显示启动画面
void display_splash_screen() {
    setlocale(LC_ALL, "");
    initscr();
    init_colors(); // Initialize colors
    cbreak();
    noecho();
    curs_set(0);

    int height, width;
    getmaxyx(stdscr, height, width);

    clear();

    // Simple ASCII art for "NBTCA Tools"
    std::string splash_art[] = {
        "███╗   ██╗██████╗ ████████╗ ██████╗ █████╗     ████████╗ ██████╗  ██████╗ ██╗     ███████╗",
        "████╗  ██║██╔══██╗╚══██╔══╝██╔════╝██╔══██╗    ╚══██╔══╝██╔═══██╗██╔═══██╗██║     ██╔════╝",
        "██╔██╗ ██║██████╔╝   ██║   ██║     ███████║       ██║   ██║   ██║██║   ██║██║     ███████╗",
        "██║╚██╗██║██╔══██╗   ██║   ██║     ██╔══██║       ██║   ██║   ██║██║   ██║██║     ╚════██║",
        "██║ ╚████║██████╔╝   ██║   ╚██████╗██║  ██║       ██║   ╚██████╔╝╚██████╔╝███████╗███████║",
        "╚═╝  ╚═══╝╚═════╝    ╚═╝    ╚═════╝╚═╝  ╚═╝       ╚═╝    ╚═════╝  ╚═════╝ ╚══════╝╚══════╝"
    };

    int art_height = sizeof(splash_art) / sizeof(splash_art[0]);
    int art_width = splash_art[0].length();

    int start_row = (height - art_height) / 2;
    int start_col = (width - art_width) / 2;

    if (start_row < 0) start_row = 0;
    if (start_col < 0) start_col = 0;

    // Draw shadow
    attron(COLOR_PAIR(SHADOW_TEXT));
    for (int i = 0; i < art_height; ++i) {
        mvprintw(start_row + i + 1, start_col + 1, "%s", splash_art[i].c_str());
    }
    attroff(COLOR_PAIR(SHADOW_TEXT));

    // Draw main art
    attron(COLOR_PAIR(BANNER_TEXT));
    for (int i = 0; i < art_height; ++i) {
        mvprintw(start_row + i, start_col, "%s", splash_art[i].c_str());
    }
    attroff(COLOR_PAIR(BANNER_TEXT));

    attron(COLOR_PAIR(NORMAL_TEXT));
    std::string loading_msg = "Loading...";
    mvprintw(start_row + art_height + 2, (width - loading_msg.length()) / 2, "%s", loading_msg.c_str());
    attroff(COLOR_PAIR(NORMAL_TEXT));

    refresh();
    std::this_thread::sleep_for(std::chrono::seconds(2)); // Display for 2 seconds

    endwin();
}


