#include "tui_view.h"

#include <curses.h>
#include <chrono>
#include <clocale>
#include <iomanip>
#include <sstream>
#include <string>
#include <vector>

namespace {

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
    cbreak();
    noecho();
    keypad(stdscr, TRUE);
    curs_set(0);

    int height, width;
    getmaxyx(stdscr, height, width);

    // 如果没有任何事件，给出提示信息
    if (events.empty()) {
        clear();
        mvprintw(0, 0, "NBTCA 未来一个月活动");
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
        mvprintw(0, 0, "NBTCA 未来一个月活动 (q 退出, ↑↓ 滚动)");

        int visibleLines = height - 2;

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
                attron(A_REVERSE);
                mvprintw(i + 1, 0, "%s", line.c_str());
                attroff(A_REVERSE);
            } else {
                mvprintw(i + 1, 0, "%s", line.c_str());
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
    cbreak();
    noecho();
    keypad(stdscr, TRUE);
    curs_set(0);

    std::vector<std::string> menu_items = {"Calendar", "Exit"};
    int selected = 0;
    int choice = -1;

    while (choice == -1) {
        clear();
        mvprintw(0, 0, "TUT - Feature Portal (q or Enter to select)");

        for (size_t i = 0; i < menu_items.size(); ++i) {
            if ((int)i == selected) {
                attron(A_REVERSE);
            }
            mvprintw(i + 2, 2, "%s", menu_items[i].c_str());
            if ((int)i == selected) {
                attroff(A_REVERSE);
            }
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


