#include "browser.h"
#include <curses.h>
#include <clocale>
#include <algorithm>
#include <sstream>
#include <map>

class Browser::Impl {
public:
    HttpClient http_client;
    HtmlParser html_parser;
    TextRenderer renderer;
    InputHandler input_handler;

    ParsedDocument current_doc;
    std::vector<RenderedLine> rendered_lines;
    std::string current_url;
    std::vector<std::string> history;
    int history_pos = -1;

    int scroll_pos = 0;
    int current_link = -1;
    std::string status_message;
    std::string search_term;
    std::vector<int> search_results;

    int screen_height = 0;
    int screen_width = 0;

    // Marks support (vim-style position bookmarks)
    std::map<char, int> marks;

    void init_screen() {
        setlocale(LC_ALL, "");
        initscr();
        init_color_scheme();
        cbreak();
        noecho();
        keypad(stdscr, TRUE);
        curs_set(0);
        timeout(0);

        // Enable mouse support
        mousemask(ALL_MOUSE_EVENTS | REPORT_MOUSE_POSITION, NULL);
        mouseinterval(0);  // No click delay

        getmaxyx(stdscr, screen_height, screen_width);
    }

    void cleanup_screen() {
        endwin();
    }

    bool load_page(const std::string& url) {
        status_message = "Loading " + url + "...";
        draw_screen();
        refresh();

        auto response = http_client.fetch(url);

        if (!response.is_success()) {
            status_message = response.error_message.empty() ?
                "HTTP " + std::to_string(response.status_code) :
                response.error_message;
            return false;
        }

        current_doc = html_parser.parse(response.body, url);
        rendered_lines = renderer.render(current_doc, screen_width);
        current_url = url;
        scroll_pos = 0;
        current_link = -1;
        search_results.clear();

        if (history_pos >= 0 && history_pos < static_cast<int>(history.size()) - 1) {
            history.erase(history.begin() + history_pos + 1, history.end());
        }
        history.push_back(url);
        history_pos = history.size() - 1;

        status_message = current_doc.title.empty() ? url : current_doc.title;
        return true;
    }

    void handle_mouse(MEVENT& event) {
        int visible_lines = screen_height - 2;

        // Mouse wheel up (scroll up)
        if (event.bstate & BUTTON4_PRESSED) {
            scroll_pos = std::max(0, scroll_pos - 3);
            return;
        }

        // Mouse wheel down (scroll down)
        if (event.bstate & BUTTON5_PRESSED) {
            int max_scroll = std::max(0, static_cast<int>(rendered_lines.size()) - visible_lines);
            scroll_pos = std::min(max_scroll, scroll_pos + 3);
            return;
        }

        // Left click
        if (event.bstate & BUTTON1_CLICKED) {
            int clicked_line = event.y;
            int clicked_col = event.x;

            // Check if clicked on a link
            if (clicked_line >= 0 && clicked_line < visible_lines) {
                int doc_line_idx = scroll_pos + clicked_line;
                if (doc_line_idx < static_cast<int>(rendered_lines.size())) {
                    const auto& line = rendered_lines[doc_line_idx];

                    // Check if click is within any link range
                    for (const auto& [start, end] : line.link_ranges) {
                        if (clicked_col >= static_cast<int>(start) && clicked_col < static_cast<int>(end)) {
                            // Clicked on a link!
                            if (line.link_index >= 0 && line.link_index < static_cast<int>(current_doc.links.size())) {
                                load_page(current_doc.links[line.link_index].url);
                                return;
                            }
                        }
                    }

                    // If clicked on a line with a link but not on the link text itself
                    if (line.is_link && line.link_index >= 0) {
                        current_link = line.link_index;
                    }
                }
            }
        }
    }

    void draw_status_bar() {
        attron(COLOR_PAIR(COLOR_STATUS_BAR));
        mvprintw(screen_height - 1, 0, "%s", std::string(screen_width, ' ').c_str());

        std::string mode_str;
        InputMode mode = input_handler.get_mode();
        switch (mode) {
            case InputMode::NORMAL:
                mode_str = "NORMAL";
                break;
            case InputMode::COMMAND:
            case InputMode::SEARCH:
                mode_str = input_handler.get_buffer();
                break;
            default:
                mode_str = "";
                break;
        }

        mvprintw(screen_height - 1, 0, " %s", mode_str.c_str());

        if (!status_message.empty() && mode == InputMode::NORMAL) {
            int msg_x = (screen_width - status_message.length()) / 2;
            if (msg_x < static_cast<int>(mode_str.length()) + 2) {
                msg_x = mode_str.length() + 2;
            }
            mvprintw(screen_height - 1, msg_x, "%s", status_message.c_str());
        }

        int total_lines = rendered_lines.size();
        int visible_lines = screen_height - 2;
        int percentage = 0;
        if (total_lines > 0) {
            if (scroll_pos == 0) {
                percentage = 0;
            } else if (scroll_pos + visible_lines >= total_lines) {
                percentage = 100;
            } else {
                percentage = (scroll_pos * 100) / total_lines;
            }
        }

        std::string pos_str = std::to_string(scroll_pos + 1) + "/" +
                             std::to_string(total_lines) + " " +
                             std::to_string(percentage) + "%";

        if (current_link >= 0 && current_link < static_cast<int>(current_doc.links.size())) {
            pos_str = "[Link " + std::to_string(current_link) + "] " + pos_str;
        }

        mvprintw(screen_height - 1, screen_width - pos_str.length() - 1, "%s", pos_str.c_str());

        attroff(COLOR_PAIR(COLOR_STATUS_BAR));
    }

    void draw_screen() {
        clear();

        int visible_lines = screen_height - 2;
        int content_lines = std::min(static_cast<int>(rendered_lines.size()) - scroll_pos, visible_lines);

        for (int i = 0; i < content_lines; ++i) {
            int line_idx = scroll_pos + i;
            const auto& line = rendered_lines[line_idx];

            // Check if this line contains the active link
            bool has_active_link = (line.is_link && line.link_index == current_link);

            // Check if this line is in search results
            bool in_search_results = !search_term.empty() &&
                std::find(search_results.begin(), search_results.end(), line_idx) != search_results.end();

            // If line has link ranges, render character by character with proper highlighting
            if (!line.link_ranges.empty()) {
                int col = 0;
                for (size_t char_idx = 0; char_idx < line.text.length(); ++char_idx) {
                    // Check if this character is within any link range
                    bool is_in_link = false;

                    for (const auto& [start, end] : line.link_ranges) {
                        if (char_idx >= start && char_idx < end) {
                            is_in_link = true;
                            break;
                        }
                    }

                    // Apply appropriate color
                    if (is_in_link && has_active_link) {
                        attron(COLOR_PAIR(COLOR_LINK_ACTIVE));
                    } else if (is_in_link) {
                        attron(COLOR_PAIR(COLOR_LINK));
                        attron(A_UNDERLINE);
                    } else {
                        attron(COLOR_PAIR(line.color_pair));
                        if (line.is_bold) {
                            attron(A_BOLD);
                        }
                    }

                    if (in_search_results) {
                        attron(A_REVERSE);
                    }

                    mvaddch(i, col, line.text[char_idx]);

                    if (in_search_results) {
                        attroff(A_REVERSE);
                    }

                    if (is_in_link && has_active_link) {
                        attroff(COLOR_PAIR(COLOR_LINK_ACTIVE));
                    } else if (is_in_link) {
                        attroff(A_UNDERLINE);
                        attroff(COLOR_PAIR(COLOR_LINK));
                    } else {
                        if (line.is_bold) {
                            attroff(A_BOLD);
                        }
                        attroff(COLOR_PAIR(line.color_pair));
                    }

                    col++;
                }
            } else {
                // No inline links, render normally
                if (has_active_link) {
                    attron(COLOR_PAIR(COLOR_LINK_ACTIVE));
                } else {
                    attron(COLOR_PAIR(line.color_pair));
                    if (line.is_bold) {
                        attron(A_BOLD);
                    }
                }

                if (in_search_results) {
                    attron(A_REVERSE);
                }

                mvprintw(i, 0, "%s", line.text.c_str());

                if (in_search_results) {
                    attroff(A_REVERSE);
                }

                if (has_active_link) {
                    attroff(COLOR_PAIR(COLOR_LINK_ACTIVE));
                } else {
                    if (line.is_bold) {
                        attroff(A_BOLD);
                    }
                    attroff(COLOR_PAIR(line.color_pair));
                }
            }
        }

        draw_status_bar();
    }

    void handle_action(const InputResult& result) {
        int visible_lines = screen_height - 2;
        int max_scroll = std::max(0, static_cast<int>(rendered_lines.size()) - visible_lines);

        int count = result.has_count ? result.count : 1;

        switch (result.action) {
            case Action::SCROLL_UP:
                scroll_pos = std::max(0, scroll_pos - count);
                break;

            case Action::SCROLL_DOWN:
                scroll_pos = std::min(max_scroll, scroll_pos + count);
                break;

            case Action::SCROLL_PAGE_UP:
                scroll_pos = std::max(0, scroll_pos - visible_lines);
                break;

            case Action::SCROLL_PAGE_DOWN:
                scroll_pos = std::min(max_scroll, scroll_pos + visible_lines);
                break;

            case Action::GOTO_TOP:
                scroll_pos = 0;
                break;

            case Action::GOTO_BOTTOM:
                scroll_pos = max_scroll;
                break;

            case Action::GOTO_LINE:
                if (result.number > 0 && result.number <= static_cast<int>(rendered_lines.size())) {
                    scroll_pos = std::min(result.number - 1, max_scroll);
                }
                break;

            case Action::NEXT_LINK:
                if (!current_doc.links.empty()) {
                    current_link = (current_link + 1) % current_doc.links.size();
                    scroll_to_link(current_link);
                }
                break;

            case Action::PREV_LINK:
                if (!current_doc.links.empty()) {
                    current_link = (current_link - 1 + current_doc.links.size()) % current_doc.links.size();
                    scroll_to_link(current_link);
                }
                break;

            case Action::FOLLOW_LINK:
                if (current_link >= 0 && current_link < static_cast<int>(current_doc.links.size())) {
                    load_page(current_doc.links[current_link].url);
                }
                break;

            case Action::GOTO_LINK:
                // Jump to specific link by number
                if (result.number >= 0 && result.number < static_cast<int>(current_doc.links.size())) {
                    current_link = result.number;
                    scroll_to_link(current_link);
                    status_message = "Link " + std::to_string(result.number);
                } else {
                    status_message = "Invalid link number: " + std::to_string(result.number);
                }
                break;

            case Action::FOLLOW_LINK_NUM:
                // Follow specific link by number directly
                if (result.number >= 0 && result.number < static_cast<int>(current_doc.links.size())) {
                    load_page(current_doc.links[result.number].url);
                } else {
                    status_message = "Invalid link number: " + std::to_string(result.number);
                }
                break;

            case Action::GO_BACK:
                if (history_pos > 0) {
                    history_pos--;
                    load_page(history[history_pos]);
                } else {
                    status_message = "No previous page";
                }
                break;

            case Action::GO_FORWARD:
                if (history_pos < static_cast<int>(history.size()) - 1) {
                    history_pos++;
                    load_page(history[history_pos]);
                } else {
                    status_message = "No next page";
                }
                break;

            case Action::OPEN_URL:
                if (!result.text.empty()) {
                    load_page(result.text);
                }
                break;

            case Action::REFRESH:
                if (!current_url.empty()) {
                    load_page(current_url);
                }
                break;

            case Action::SEARCH_FORWARD:
                search_term = result.text;
                search_results.clear();
                for (size_t i = 0; i < rendered_lines.size(); ++i) {
                    if (rendered_lines[i].text.find(search_term) != std::string::npos) {
                        search_results.push_back(i);
                    }
                }
                if (!search_results.empty()) {
                    scroll_pos = search_results[0];
                    status_message = "Found " + std::to_string(search_results.size()) + " matches";
                } else {
                    status_message = "Pattern not found: " + search_term;
                }
                break;

            case Action::SEARCH_NEXT:
                if (!search_results.empty()) {
                    auto it = std::upper_bound(search_results.begin(), search_results.end(), scroll_pos);
                    if (it != search_results.end()) {
                        scroll_pos = *it;
                    } else {
                        scroll_pos = search_results[0];
                        status_message = "Search wrapped to top";
                    }
                }
                break;

            case Action::SEARCH_PREV:
                if (!search_results.empty()) {
                    auto it = std::lower_bound(search_results.begin(), search_results.end(), scroll_pos);
                    if (it != search_results.begin()) {
                        scroll_pos = *(--it);
                    } else {
                        scroll_pos = search_results.back();
                        status_message = "Search wrapped to bottom";
                    }
                }
                break;

            case Action::SET_MARK:
                if (!result.text.empty()) {
                    char mark = result.text[0];
                    marks[mark] = scroll_pos;
                    status_message = "Mark '" + std::string(1, mark) + "' set at line " + std::to_string(scroll_pos);
                }
                break;

            case Action::GOTO_MARK:
                if (!result.text.empty()) {
                    char mark = result.text[0];
                    auto it = marks.find(mark);
                    if (it != marks.end()) {
                        scroll_pos = std::min(it->second, max_scroll);
                        status_message = "Jumped to mark '" + std::string(1, mark) + "'";
                    } else {
                        status_message = "Mark '" + std::string(1, mark) + "' not set";
                    }
                }
                break;

            case Action::HELP:
                show_help();
                break;

            default:
                break;
        }
    }

    void scroll_to_link(int link_idx) {
        for (size_t i = 0; i < rendered_lines.size(); ++i) {
            if (rendered_lines[i].is_link && rendered_lines[i].link_index == link_idx) {
                int visible_lines = screen_height - 2;
                if (static_cast<int>(i) < scroll_pos || static_cast<int>(i) >= scroll_pos + visible_lines) {
                    scroll_pos = std::max(0, static_cast<int>(i) - visible_lines / 2);
                }
                break;
            }
        }
    }

    void show_help() {
        std::ostringstream help_html;
        help_html << "<html><head><title>TUT Browser Help</title></head><body>"
                  << "<h1>TUT Browser - Vim-style Terminal Browser</h1>"
                  << "<h2>Navigation</h2>"
                  << "<p>j/k or ↓/↑: Scroll down/up</p>"
                  << "<p>Ctrl-D or Space: Scroll page down</p>"
                  << "<p>Ctrl-U or b: Scroll page up</p>"
                  << "<p>gg: Go to top</p>"
                  << "<p>G: Go to bottom</p>"
                  << "<p>[number]G: Go to line number</p>"
                  << "<h2>Links</h2>"
                  << "<p>Links are displayed inline with numbers like [0], [1], etc.</p>"
                  << "<p>Tab: Next link</p>"
                  << "<p>Shift-Tab or T: Previous link</p>"
                  << "<p>Enter: Follow current link</p>"
                  << "<p>[number]Enter: Jump to link number N</p>"
                  << "<p>f[number]: Follow link number N directly</p>"
                  << "<p>h: Go back</p>"
                  << "<p>l: Go forward</p>"
                  << "<h2>Search</h2>"
                  << "<p>/: Start search</p>"
                  << "<p>n: Next match</p>"
                  << "<p>N: Previous match</p>"
                  << "<h2>Commands</h2>"
                  << "<p>:q or :quit - Quit browser</p>"
                  << "<p>:o URL or :open URL - Open URL</p>"
                  << "<p>:r or :refresh - Refresh page</p>"
                  << "<p>:h or :help - Show this help</p>"
                  << "<p>:[number] - Go to line number</p>"
                  << "<h2>Marks</h2>"
                  << "<p>m[a-z]: Set mark at letter (e.g., ma, mb)</p>"
                  << "<p>'[a-z]: Jump to mark (e.g., 'a, 'b)</p>"
                  << "<h2>Mouse Support</h2>"
                  << "<p>Click on links to follow them</p>"
                  << "<p>Scroll wheel to scroll up/down</p>"
                  << "<p>Works with most terminal emulators</p>"
                  << "<h2>Other</h2>"
                  << "<p>r: Refresh current page</p>"
                  << "<p>q: Quit browser</p>"
                  << "<p>?: Show help</p>"
                  << "<p>ESC: Cancel current mode</p>"
                  << "<h2>Important Limitations</h2>"
                  << "<p><strong>JavaScript/SPA Websites:</strong> This browser cannot execute JavaScript. "
                  << "Single Page Applications (SPAs) built with React, Vue, Angular, etc. will not work properly "
                  << "as they render content dynamically with JavaScript.</p>"
                  << "<p><strong>Works best with:</strong></p>"
                  << "<ul>"
                  << "<li>Static HTML websites</li>"
                  << "<li>Server-side rendered pages</li>"
                  << "<li>Documentation sites</li>"
                  << "<li>News sites with HTML content</li>"
                  << "<li>Blogs with traditional HTML</li>"
                  << "</ul>"
                  << "<p><strong>Example sites that work well:</strong></p>"
                  << "<p>- https://example.com</p>"
                  << "<p>- https://en.wikipedia.org</p>"
                  << "<p>- Text-based news sites</p>"
                  << "<p><strong>For JavaScript-heavy sites:</strong> You may need to find alternative URLs "
                  << "that provide the same content in plain HTML format.</p>"
                  << "</body></html>";

        current_doc = html_parser.parse(help_html.str(), "help://");
        rendered_lines = renderer.render(current_doc, screen_width);
        scroll_pos = 0;
        current_link = -1;
        status_message = "Help - Press q to return";
    }
};

Browser::Browser() : pImpl(std::make_unique<Impl>()) {
    pImpl->input_handler.set_status_callback([this](const std::string& msg) {
        pImpl->status_message = msg;
    });
}

Browser::~Browser() = default;

void Browser::run(const std::string& initial_url) {
    pImpl->init_screen();

    if (!initial_url.empty()) {
        load_url(initial_url);
    } else {
        pImpl->show_help();
    }

    bool running = true;
    while (running) {
        pImpl->draw_screen();
        refresh();

        int ch = getch();
        if (ch == ERR) {
            napms(50);
            continue;
        }

        // Handle mouse events
        if (ch == KEY_MOUSE) {
            MEVENT event;
            if (getmouse(&event) == OK) {
                pImpl->handle_mouse(event);
            }
            continue;
        }

        auto result = pImpl->input_handler.handle_key(ch);

        if (result.action == Action::QUIT) {
            running = false;
        } else if (result.action != Action::NONE) {
            pImpl->handle_action(result);
        }
    }

    pImpl->cleanup_screen();
}

bool Browser::load_url(const std::string& url) {
    return pImpl->load_page(url);
}

std::string Browser::get_current_url() const {
    return pImpl->current_url;
}
