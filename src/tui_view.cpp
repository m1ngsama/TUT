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

// Define color pairs - Modern btop-inspired color scheme
enum ColorPairs {
    NORMAL_TEXT = 1,        // Default white text
    SHADOW_TEXT,            // Dim shadow text
    BANNER_TEXT,            // Bright cyan for banners
    SELECTED_ITEM,          // Bright yellow for selected items
    BORDER_LINE,            // Gray borders and boxes
    SUCCESS_TEXT,           // Green for success states
    WARNING_TEXT,           // Orange/yellow for warnings
    ERROR_TEXT,             // Red for errors
    INFO_TEXT,              // Blue for information
    ACCENT_TEXT,            // Magenta for accents
    DIM_TEXT,               // Dimmed secondary text
    PROGRESS_BAR,           // Green progress bars
    CALENDAR_HEADER,        // Calendar header styling
    EVENT_PAST,             // Grayed out past events
    EVENT_TODAY,            // Highlighted today's events
    EVENT_UPCOMING          // Default upcoming events
};

void init_colors() {
    if (has_colors()) {
        start_color();
        use_default_colors(); // Use terminal's default background

        // Modern color scheme inspired by btop
        init_pair(NORMAL_TEXT, COLOR_WHITE, -1);           // White text
        init_pair(SHADOW_TEXT, COLOR_BLACK, -1);           // Black shadow text
        init_pair(BANNER_TEXT, COLOR_CYAN, -1);            // Bright cyan for banners
        init_pair(SELECTED_ITEM, COLOR_YELLOW, -1);        // Bright yellow selection
        init_pair(BORDER_LINE, COLOR_BLUE, -1);            // Blue borders/boxes
        init_pair(SUCCESS_TEXT, COLOR_GREEN, -1);          // Green for success
        init_pair(WARNING_TEXT, COLOR_YELLOW, -1);         // Orange/yellow warnings
        init_pair(ERROR_TEXT, COLOR_RED, -1);              // Red for errors
        init_pair(INFO_TEXT, COLOR_BLUE, -1);              // Blue for info
        init_pair(ACCENT_TEXT, COLOR_MAGENTA, -1);         // Magenta accents
        init_pair(DIM_TEXT, COLOR_BLACK, -1);              // Dimmed text
        init_pair(PROGRESS_BAR, COLOR_GREEN, -1);          // Green progress
        init_pair(CALENDAR_HEADER, COLOR_CYAN, -1);        // Calendar headers
        init_pair(EVENT_PAST, COLOR_BLACK, -1);            // Grayed past events
        init_pair(EVENT_TODAY, COLOR_YELLOW, -1);          // Today's events highlighted
        init_pair(EVENT_UPCOMING, COLOR_WHITE, -1);        // Default upcoming events
    }
}

// Helper function to draw a box
void draw_box(int start_y, int start_x, int width, int height, bool shadow = false) {
    if (shadow) {
        // Draw shadow first
        attron(COLOR_PAIR(SHADOW_TEXT));
        for (int i = 0; i < height; i++) {
            mvprintw(start_y + i + 1, start_x + 1, "%s", std::string(width, ' ').c_str());
        }
        attroff(COLOR_PAIR(SHADOW_TEXT));
    }

    attron(COLOR_PAIR(BORDER_LINE));
    // Draw corners
    mvprintw(start_y, start_x, "┌");
    mvprintw(start_y, start_x + width - 1, "┐");
    mvprintw(start_y + height - 1, start_x, "└");
    mvprintw(start_y + height - 1, start_x + width - 1, "┘");

    // Draw horizontal lines
    for (int i = 1; i < width - 1; i++) {
        mvprintw(start_y, start_x + i, "─");
        mvprintw(start_y + height - 1, start_x + i, "─");
    }

    // Draw vertical lines
    for (int i = 1; i < height - 1; i++) {
        mvprintw(start_y + i, start_x, "│");
        mvprintw(start_y + i, start_x + width - 1, "│");
    }
    attroff(COLOR_PAIR(BORDER_LINE));
}

// Helper function to draw a progress bar
void draw_progress_bar(int y, int x, int width, float percentage) {
    int filled_width = static_cast<int>(width * percentage);

    attron(COLOR_PAIR(BORDER_LINE));
    mvprintw(y, x, "[");
    mvprintw(y, x + width - 1, "]");
    attroff(COLOR_PAIR(BORDER_LINE));

    attron(COLOR_PAIR(PROGRESS_BAR));
    for (int i = 1; i < filled_width && i < width - 1; i++) {
        mvprintw(y, x + i, "█");
    }
    attroff(COLOR_PAIR(PROGRESS_BAR));
}

// Helper function to center text within a box
void draw_centered_text(int y, int box_start_x, int box_width, const std::string& text, int color_pair = NORMAL_TEXT) {
    int text_x = box_start_x + (box_width - text.length()) / 2;
    if (text_x < box_start_x) text_x = box_start_x;

    attron(COLOR_PAIR(color_pair));
    mvprintw(y, text_x, "%s", text.c_str());
    attroff(COLOR_PAIR(color_pair));
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

// Helper function to check if event is today, past, or upcoming
int get_event_status(const std::chrono::system_clock::time_point &event_time) {
    auto now = std::chrono::system_clock::now();

    if (event_time < now) {
        return EVENT_PAST;
    } else {
        // Simple check if it's today (within 24 hours)
        auto hours_until_event = std::chrono::duration_cast<std::chrono::hours>(event_time - now);
        if (hours_until_event.count() <= 24) {
            return EVENT_TODAY;
        } else {
            return EVENT_UPCOMING;
        }
    }
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

        // Modern ASCII art banner for "CALENDAR" - smaller and cleaner
        std::string calendar_banner[] = {
            "  ╔═══════════════════════════════════╗  ",
            "  ║  [CAL] NBTCA CALENDAR [CAL]  ║  ",
            "  ╚═══════════════════════════════════╝  "
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

        // Draw status bar with current date and event count
        attron(COLOR_PAIR(BORDER_LINE));
        mvprintw(banner_height + 1, 0, "┌");
        mvprintw(banner_height + 1, width - 1, "┐");
        for (int i = 1; i < width - 1; i++) {
            mvprintw(banner_height + 1, i, "─");
        }
        attroff(COLOR_PAIR(BORDER_LINE));

        // Status information
        auto now = std::chrono::system_clock::now();
        auto tt = std::chrono::system_clock::to_time_t(now);
        std::tm tm{};
#if defined(_WIN32)
        localtime_s(&tm, &tt);
#else
        localtime_r(&tt, &tm);
#endif
        std::ostringstream oss;
        oss << std::put_time(&tm, "%Y-%m-%d %A");
        std::string current_date = "Today: " + oss.str() + " | Events: " + std::to_string(events.size());

        attron(COLOR_PAIR(INFO_TEXT));
        mvprintw(banner_height + 1, 2, "%s", current_date.c_str());
        attroff(COLOR_PAIR(INFO_TEXT));

        attron(COLOR_PAIR(DIM_TEXT));
        std::string instruction_msg = "[q:Exit ↑↓:Scroll]";
        mvprintw(banner_height + 1, width - instruction_msg.length() - 2, "%s", instruction_msg.c_str());
        attroff(COLOR_PAIR(DIM_TEXT));

        int start_event_row = banner_height + 3; // Start events below the banner and instruction
        int visibleLines = height - start_event_row - 2; // Leave space for bottom border

        // Draw main event container box
        draw_box(start_event_row - 1, 0, width, visibleLines + 2, true);

        // Header for events list
        attron(COLOR_PAIR(CALENDAR_HEADER));
        mvprintw(start_event_row, 2, "╓ Upcoming Events");
        attroff(COLOR_PAIR(CALENDAR_HEADER));

        int events_start_row = start_event_row + 1;
        int events_visible_lines = visibleLines - 2;

        if (selected < top) {
            top = selected;
        } else if (selected >= top + events_visible_lines) {
            top = selected - events_visible_lines + 1;
        }

        for (int i = 0; i < events_visible_lines; ++i) {
            int idx = top + i;
            if (idx >= static_cast<int>(events.size())) break;

            const auto &ev = events[idx];
            int event_status = get_event_status(ev.start);

            // Event icon based on status
            std::string icon = "○";
            if (event_status == EVENT_TODAY) {
                icon = "*";
            } else if (event_status == EVENT_PAST) {
                icon = "v";
            }

            // Format date more compactly
            auto tt = std::chrono::system_clock::to_time_t(ev.start);
            std::tm tm{};
#if defined(_WIN32)
            localtime_s(&tm, &tt);
#else
            localtime_r(&tt, &tm);
#endif
            std::ostringstream oss;
            oss << std::put_time(&tm, "%m/%d %H:%M");
            std::string date_str = oss.str();

            // Build the event line
            std::string line = icon + " " + date_str + " " + ev.summary;
            if (!ev.location.empty()) {
                line += " @" + ev.location;
            }

            // Truncate if too long
            if ((int)line.length() > width - 4) {
                line.resize(width - 4);
            }

            // Determine colors based on event status and selection
            int text_color = (event_status == EVENT_PAST) ? DIM_TEXT :
                           (event_status == EVENT_TODAY) ? EVENT_TODAY :
                           EVENT_UPCOMING;

            if (idx == selected) {
                attron(A_REVERSE | COLOR_PAIR(SELECTED_ITEM));
                mvprintw(events_start_row + i, 2, "%s", std::string(width - 4, ' ').c_str());
                mvprintw(events_start_row + i, 3, "%s", line.c_str());
                attroff(A_REVERSE | COLOR_PAIR(SELECTED_ITEM));
            } else {
                attron(COLOR_PAIR(text_color));
                mvprintw(events_start_row + i, 3, "%s", line.c_str());
                attroff(COLOR_PAIR(text_color));
            }

            // Add a subtle separator
            if (i < events_visible_lines - 1 && idx + 1 < static_cast<int>(events.size())) {
                attron(COLOR_PAIR(BORDER_LINE));
                mvprintw(events_start_row + i + 1, 2, "├");
                for (int j = 3; j < width - 2; j++) {
                    mvprintw(events_start_row + i + 1, j, "─");
                }
                mvprintw(events_start_row + i + 1, width - 2, "┤");
                attroff(COLOR_PAIR(BORDER_LINE));
            }
        }

        // Add scroll indicator if there are more events
        if (events.size() > events_visible_lines) {
            float scroll_percentage = static_cast<float>(top + events_visible_lines) / events.size();
            int scroll_y = start_event_row + 1;
            int scroll_x = width - 3;
            draw_progress_bar(scroll_y, scroll_x, 1, scroll_percentage);
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

        // Modern ASCII art banner for "NBTCA Tools" - smaller and cleaner
        std::string banner_art[] = {
            "  ╔══════════════════════════════════════╗  ",
            "  ║ [TOOL] NBTCA UTILITY TOOLS [TOOL] ║  ",
            "  ╚══════════════════════════════════════╝  "
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

        // Draw info bar
        attron(COLOR_PAIR(BORDER_LINE));
        mvprintw(banner_height + 1, 0, "┌");
        mvprintw(banner_height + 1, width - 1, "┐");
        for (int i = 1; i < width - 1; i++) {
            mvprintw(banner_height + 1, i, "─");
        }
        attroff(COLOR_PAIR(BORDER_LINE));

        // Status information
        auto now = std::chrono::system_clock::now();
        auto tt = std::chrono::system_clock::to_time_t(now);
        std::tm tm{};
#if defined(_WIN32)
        localtime_s(&tm, &tt);
#else
        localtime_r(&tt, &tm);
#endif
        std::ostringstream oss;
        oss << std::put_time(&tm, "%Y-%m-%d %H:%M:%S");
        std::string current_time = "Current: " + oss.str();

        attron(COLOR_PAIR(INFO_TEXT));
        mvprintw(banner_height + 1, 2, "%s", current_time.c_str());
        attroff(COLOR_PAIR(INFO_TEXT));

        attron(COLOR_PAIR(DIM_TEXT));
        std::string help_msg = "[↑↓:Navigate Enter:Select q:Exit]";
        mvprintw(banner_height + 1, width - help_msg.length() - 2, "%s", help_msg.c_str());
        attroff(COLOR_PAIR(DIM_TEXT));

        // Draw menu box
        int menu_box_y = banner_height + 3;
        int menu_box_height = menu_items.size() + 4;
        int menu_box_width = 30;
        int menu_box_x = (width - menu_box_width) / 2;
        if (menu_box_x < 2) menu_box_x = 2;

        draw_box(menu_box_y, menu_box_x, menu_box_width, menu_box_height, true);

        // Menu box title
        attron(COLOR_PAIR(CALENDAR_HEADER));
        draw_centered_text(menu_box_y + 1, menu_box_x, menu_box_width, "Select Module");
        attroff(COLOR_PAIR(CALENDAR_HEADER));

        // Draw menu items with icons
        for (size_t i = 0; i < menu_items.size(); ++i) {
            std::string display_item = menu_items[i];

            // Add icons to menu items
            if (display_item == "Calendar") {
                display_item = "[CAL] " + display_item;
            } else if (display_item == "Exit") {
                display_item = "[X] " + display_item;
            }

            int item_y = menu_box_y + 2 + i;

            if ((int)i == selected) {
                // Draw selection highlight box
                attron(COLOR_PAIR(BORDER_LINE));
                mvprintw(item_y, menu_box_x + 1, "│");
                mvprintw(item_y, menu_box_x + menu_box_width - 2, "│");
                attroff(COLOR_PAIR(BORDER_LINE));

                attron(A_REVERSE | COLOR_PAIR(SELECTED_ITEM));
                draw_centered_text(item_y, menu_box_x, menu_box_width, display_item, SELECTED_ITEM);
                attroff(A_REVERSE | COLOR_PAIR(SELECTED_ITEM));
            } else {
                attron(COLOR_PAIR(NORMAL_TEXT));
                draw_centered_text(item_y, menu_box_x, menu_box_width, display_item);
                attroff(COLOR_PAIR(NORMAL_TEXT));
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

    // Animated splash screen with progress bar
    for (int frame = 0; frame < 20; ++frame) {
        clear();

        // Modern ASCII art for "NBTCA Tools" - cleaner design
        std::string splash_art[] = {
            "  ╔══════════════════════════════════════╗  ",
            "  ║ [TOOL] NBTCA UTILITY TOOLS [TOOL] ║  ",
            "  ╚══════════════════════════════════════╝  "
        };

        int art_height = sizeof(splash_art) / sizeof(splash_art[0]);
        int art_width = splash_art[0].length();

        int start_row = (height - art_height) / 2 - 3;
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

        // Version info
        attron(COLOR_PAIR(INFO_TEXT));
        draw_centered_text(start_row + art_height + 1, start_col, art_width, "Version 0.0.1");
        attroff(COLOR_PAIR(INFO_TEXT));

        // Loading text with rotating animation
        const char* spinner[] = {"|", "/", "-", "\\"};
        std::string loading_msg = std::string(spinner[frame % 4]) + " Initializing system components...";

        attron(COLOR_PAIR(NORMAL_TEXT));
        draw_centered_text(start_row + art_height + 3, start_col, art_width, loading_msg);
        attroff(COLOR_PAIR(NORMAL_TEXT));

        // Progress bar
        int progress_bar_y = start_row + art_height + 5;
        int progress_bar_width = 40;
        int progress_bar_x = (width - progress_bar_width) / 2;

        float progress = static_cast<float>(frame) / 19.0f;
        draw_progress_bar(progress_bar_y, progress_bar_x, progress_bar_width, progress);

        // Progress percentage
        attron(COLOR_PAIR(SUCCESS_TEXT));
        std::string progress_text = std::to_string(static_cast<int>(progress * 100)) + "% Complete";
        draw_centered_text(progress_bar_y + 1, progress_bar_x, progress_bar_width, progress_text);
        attroff(COLOR_PAIR(SUCCESS_TEXT));

        // Status messages
        std::vector<std::string> status_msgs = {
            "Loading calendar module...",
            "Initializing network stack...",
            "Fetching latest events...",
            "Preparing user interface...",
            "System ready!"
        };

        int msg_index = (frame * status_msgs.size()) / 20;
        if (msg_index >= status_msgs.size()) msg_index = status_msgs.size() - 1;

        attron(COLOR_PAIR(DIM_TEXT));
        draw_centered_text(progress_bar_y + 2, progress_bar_x, progress_bar_width, status_msgs[msg_index]);
        attroff(COLOR_PAIR(DIM_TEXT));

        refresh();
        std::this_thread::sleep_for(std::chrono::milliseconds(100)); // 100ms per frame
    }

    endwin();
}


