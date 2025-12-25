#include "browser.h"
#include "dom_tree.h"
#include <curses.h>
#include <clocale>
#include <algorithm>
#include <sstream>
#include <map>
#include <cctype>
#include <cstdio>

class Browser::Impl {
public:
    HttpClient http_client;
    HtmlParser html_parser;
    TextRenderer renderer;
    InputHandler input_handler;

    DocumentTree current_tree;
    std::vector<RenderedLine> rendered_lines;
    std::string current_url;
    std::vector<std::string> history;
    int history_pos = -1;

    int scroll_pos = 0;
    std::string status_message;
    std::string search_term;
    std::vector<int> search_results;

    int screen_height = 0;
    int screen_width = 0;

    // Marks support
    std::map<char, int> marks;

    // Interactive elements (Links + Form Fields)
    struct InteractiveElement {
        int link_index = -1;
        int field_index = -1;
        int line_index = -1;
        InteractiveRange range;
    };
    std::vector<InteractiveElement> interactive_elements;
    int current_element_index = -1;

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

    void build_interactive_list() {
        interactive_elements.clear();
        for (size_t i = 0; i < rendered_lines.size(); ++i) {
            for (const auto& range : rendered_lines[i].interactive_ranges) {
                InteractiveElement el;
                el.link_index = range.link_index;
                el.field_index = range.field_index;
                el.line_index = static_cast<int>(i);
                el.range = range;
                interactive_elements.push_back(el);
            }
        }
        
        // Reset or adjust current_element_index
        if (current_element_index >= static_cast<int>(interactive_elements.size())) {
            current_element_index = interactive_elements.empty() ? -1 : 0;
        }
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

        current_tree = html_parser.parse_tree(response.body, url);
        rendered_lines = renderer.render_tree(current_tree, screen_width);
        build_interactive_list();
        
        current_url = url;
        scroll_pos = 0;
        current_element_index = interactive_elements.empty() ? -1 : 0;
        search_results.clear();

        if (history_pos >= 0 && history_pos < static_cast<int>(history.size()) - 1) {
            history.erase(history.begin() + history_pos + 1, history.end());
        }
        history.push_back(url);
        history_pos = history.size() - 1;

        status_message = current_tree.title.empty() ? url : current_tree.title;
        return true;
    }

    void handle_mouse(MEVENT& event) {
        int visible_lines = screen_height - 2;

        if (event.bstate & BUTTON4_PRESSED) {
            scroll_pos = std::max(0, scroll_pos - 3);
            return;
        }

        if (event.bstate & BUTTON5_PRESSED) {
            int max_scroll = std::max(0, static_cast<int>(rendered_lines.size()) - visible_lines);
            scroll_pos = std::min(max_scroll, scroll_pos + 3);
            return;
        }

        if (event.bstate & BUTTON1_CLICKED) {
            int clicked_line = event.y;
            int clicked_col = event.x;

            if (clicked_line >= 0 && clicked_line < visible_lines) {
                int doc_line_idx = scroll_pos + clicked_line;
                if (doc_line_idx < static_cast<int>(rendered_lines.size())) {
                    for (size_t i = 0; i < interactive_elements.size(); ++i) {
                        const auto& el = interactive_elements[i];
                        if (el.line_index == doc_line_idx && 
                            clicked_col >= static_cast<int>(el.range.start) && 
                            clicked_col < static_cast<int>(el.range.end)) {
                            
                            current_element_index = i;
                            activate_element(i);
                            return;
                        }
                    }
                }
            }
        }
    }

    void activate_element(int index) {
        if (index < 0 || index >= static_cast<int>(interactive_elements.size())) return;
        
        const auto& el = interactive_elements[index];
        if (el.link_index >= 0) {
            if (el.link_index < static_cast<int>(current_tree.links.size())) {
                load_page(current_tree.links[el.link_index].url);
            }
        } else if (el.field_index >= 0) {
            handle_form_interaction(el.field_index);
        }
    }

    void handle_form_interaction(int field_idx) {
        if (field_idx < 0 || field_idx >= static_cast<int>(current_tree.form_fields.size())) return;
        
        DomNode* node = current_tree.form_fields[field_idx];
        
        if (node->input_type == "checkbox" || node->input_type == "radio") {
            if (node->input_type == "radio") {
                // Uncheck others in same group
                DomNode* form = node->parent;
                // Find form parent
                while (form && form->element_type != ElementType::FORM) form = form->parent;
                
                // If found form, traverse to uncheck others with same name
                // This is a complex traversal, simplified: just toggle for now or assume single radio group
                node->checked = true; 
            } else {
                node->checked = !node->checked;
            }
            // Re-render
            rendered_lines = renderer.render_tree(current_tree, screen_width);
            build_interactive_list();
        } else if (node->input_type == "text" || node->input_type == "password" || 
                   node->input_type == "textarea" || node->input_type == "search" ||
                   node->input_type == "email" || node->input_type == "url") {
            
            // Prompt user
            mvprintw(screen_height - 1, 0, "Input: ");
            clrtoeol();
            echo();
            curs_set(1);
            char buffer[256];
            getnstr(buffer, 255);
            noecho();
            curs_set(0);
            
            node->value = buffer;
            rendered_lines = renderer.render_tree(current_tree, screen_width);
            build_interactive_list();
            
        } else if (node->input_type == "submit" || node->input_type == "button") {
            submit_form(node);
        }
    }

    // URL encode helper function
    std::string url_encode(const std::string& value) {
        std::string result;
        for (unsigned char c : value) {
            if (isalnum(c) || c == '-' || c == '_' || c == '.' || c == '~') {
                result += c;
            } else if (c == ' ') {
                result += '+';
            } else {
                char hex[4];
                snprintf(hex, sizeof(hex), "%%%02X", c);
                result += hex;
            }
        }
        return result;
    }

    void submit_form(DomNode* button) {
        status_message = "Submitting form...";

        // Find parent form
        DomNode* form = button->parent;
        while (form && form->element_type != ElementType::FORM) form = form->parent;

        if (!form) {
            status_message = "Error: Button not in a form";
            return;
        }

        // Collect form data with URL encoding
        std::string form_data;
        for (DomNode* field : current_tree.form_fields) {
            // Check if field belongs to this form
            DomNode* p = field->parent;
            bool is_child = false;
            while(p) { if(p == form) { is_child = true; break; } p = p->parent; }

            if (is_child && !field->name.empty()) {
                if (!form_data.empty()) form_data += "&";
                form_data += url_encode(field->name) + "=" + url_encode(field->value);
            }
        }

        std::string target_url = form->action;
        if (target_url.empty()) target_url = current_url;

        // Check form method (default to GET if not specified)
        std::string method = form->method;
        std::transform(method.begin(), method.end(), method.begin(), ::toupper);

        if (method == "POST") {
            // POST request
            status_message = "Sending POST request...";
            HttpResponse response = http_client.post(target_url, form_data);

            if (!response.error_message.empty()) {
                status_message = "Error: " + response.error_message;
                return;
            }

            if (!response.is_success()) {
                status_message = "Error: HTTP " + std::to_string(response.status_code);
                return;
            }

            // Parse and render response
            DocumentTree tree = html_parser.parse_tree(response.body, target_url);
            current_tree = std::move(tree);
            current_url = target_url;
            rendered_lines = renderer.render_tree(current_tree, screen_width);
            build_interactive_list();
            scroll_pos = 0;
            current_element_index = -1;

            // Update history
            if (history_pos < static_cast<int>(history.size()) - 1) {
                history.erase(history.begin() + history_pos + 1, history.end());
            }
            history.push_back(current_url);
            history_pos = history.size() - 1;

            status_message = "Form submitted (POST)";
        } else {
            // GET request (default)
            if (target_url.find('?') == std::string::npos) {
                target_url += "?" + form_data;
            } else {
                target_url += "&" + form_data;
            }
            load_page(target_url);
            status_message = "Form submitted (GET)";
        }
    }

    void draw_status_bar() {
        attron(COLOR_PAIR(COLOR_STATUS_BAR));
        mvprintw(screen_height - 1, 0, "%s", std::string(screen_width, ' ').c_str());

        std::string mode_str;
        InputMode mode = input_handler.get_mode();
        switch (mode) {
            case InputMode::NORMAL: mode_str = "NORMAL"; break;
            case InputMode::COMMAND:
            case InputMode::SEARCH: mode_str = input_handler.get_buffer(); break;
            default: mode_str = ""; break;
        }

        mvprintw(screen_height - 1, 0, " %s", mode_str.c_str());

        if (mode == InputMode::NORMAL) {
            std::string display_msg;
            
            // Priority: Hovered Link URL > Status Message > Title
            if (current_element_index >= 0 && 
                current_element_index < static_cast<int>(interactive_elements.size())) {
                const auto& el = interactive_elements[current_element_index];
                if (el.link_index >= 0 && el.link_index < static_cast<int>(current_tree.links.size())) {
                    display_msg = current_tree.links[el.link_index].url;
                }
            }
            
            if (display_msg.empty()) {
                display_msg = status_message;
            }

            if (!display_msg.empty()) {
                int msg_x = (screen_width - display_msg.length()) / 2;
                if (msg_x < static_cast<int>(mode_str.length()) + 2) msg_x = mode_str.length() + 2;
                // Truncate if too long
                int max_len = screen_width - msg_x - 20; // Reserve space for position info
                if (max_len > 0) {
                    if (display_msg.length() > static_cast<size_t>(max_len)) {
                        display_msg = display_msg.substr(0, max_len - 3) + "...";
                    }
                    mvprintw(screen_height - 1, msg_x, "%s", display_msg.c_str());
                }
            }
        }

        int total_lines = rendered_lines.size();
        int percentage = (total_lines > 0 && scroll_pos + screen_height - 2 < total_lines) ? 
                         (scroll_pos * 100) / total_lines : 100;
        if (total_lines == 0) percentage = 0;

        std::string pos_str = std::to_string(scroll_pos + 1) + "/" + std::to_string(total_lines) + " " + std::to_string(percentage) + "%";
        mvprintw(screen_height - 1, screen_width - pos_str.length() - 1, "%s", pos_str.c_str());

        attroff(COLOR_PAIR(COLOR_STATUS_BAR));
    }

    int get_utf8_sequence_length(char c) {
        if ((c & 0x80) == 0) return 1;
        if ((c & 0xE0) == 0xC0) return 2;
        if ((c & 0xF0) == 0xE0) return 3;
        if ((c & 0xF8) == 0xF0) return 4;
        return 1; // Fallback
    }

    void draw_screen() {
        clear();

        int visible_lines = screen_height - 2;
        int content_lines = std::min(static_cast<int>(rendered_lines.size()) - scroll_pos, visible_lines);
        
        int cursor_y = -1;
        int cursor_x = -1;

        for (int i = 0; i < content_lines; ++i) {
            int line_idx = scroll_pos + i;
            const auto& line = rendered_lines[line_idx];

            // Check if this line is in search results
            bool in_search_results = !search_term.empty() &&
                std::find(search_results.begin(), search_results.end(), line_idx) != search_results.end();

            move(i, 0); // Move to start of line

            size_t byte_idx = 0;
            int current_col = 0; // Track visual column
            
            while (byte_idx < line.text.length()) {
                size_t seq_len = get_utf8_sequence_length(line.text[byte_idx]);
                // Ensure we don't read past end of string (malformed utf8 protection)
                if (byte_idx + seq_len > line.text.length()) {
                    seq_len = line.text.length() - byte_idx;
                }

                bool is_active = false;
                bool is_interactive = false;
                
                // Check if current byte position falls within an interactive range
                for (const auto& range : line.interactive_ranges) {
                    if (byte_idx >= range.start && byte_idx < range.end) {
                        is_interactive = true;
                        // Check if this is the currently selected element
                        if (current_element_index >= 0 && 
                            current_element_index < static_cast<int>(interactive_elements.size())) {
                            const auto& el = interactive_elements[current_element_index];
                            if (el.line_index == line_idx && 
                                el.range.start == range.start && 
                                el.range.end == range.end) {
                                is_active = true;
                                // Capture cursor position for the START of the active element
                                if (byte_idx == range.start && cursor_y == -1) {
                                    cursor_y = i;
                                    cursor_x = current_col;
                                }
                            }
                        }
                        break;
                    }
                }

                // Apply attributes
                if (is_active) {
                    attron(COLOR_PAIR(COLOR_LINK_ACTIVE));
                } else if (is_interactive) {
                    attron(COLOR_PAIR(COLOR_LINK));
                    attron(A_UNDERLINE);
                } else {
                    attron(COLOR_PAIR(line.color_pair));
                    if (line.is_bold) attron(A_BOLD);
                }

                if (in_search_results) attron(A_REVERSE);

                // Print the UTF-8 sequence
                addnstr(line.text.c_str() + byte_idx, seq_len);
                
                // Approximate column width update (simple)
                // For proper handling, we should use wcwidth, but for now assuming 1 or 2 based on seq_len is "okay" approximation for cursor placement
                // actually addnstr advances cursor, getyx is better?
                // But we are in a loop.
                int unused_y, x;
                getyx(stdscr, unused_y, x);
                (void)unused_y;  // Suppress unused variable warning
                current_col = x;

                // Clear attributes
                if (in_search_results) attroff(A_REVERSE);

                if (is_active) {
                    attroff(COLOR_PAIR(COLOR_LINK_ACTIVE));
                } else if (is_interactive) {
                    attroff(A_UNDERLINE);
                    attroff(COLOR_PAIR(COLOR_LINK));
                } else {
                    if (line.is_bold) attroff(A_BOLD);
                    attroff(COLOR_PAIR(line.color_pair));
                }

                byte_idx += seq_len;
            }
        }

        draw_status_bar();
        
        // Place cursor
        if (cursor_y != -1 && cursor_x != -1) {
            curs_set(1);
            move(cursor_y, cursor_x);
        } else {
            curs_set(0);
        }
    }

    void handle_action(const InputResult& result) {
        int visible_lines = screen_height - 2;
        int max_scroll = std::max(0, static_cast<int>(rendered_lines.size()) - visible_lines);
        int count = result.has_count ? result.count : 1;

        switch (result.action) {
            case Action::SCROLL_UP: scroll_pos = std::max(0, scroll_pos - count); break;
            case Action::SCROLL_DOWN: scroll_pos = std::min(max_scroll, scroll_pos + count); break;
            case Action::SCROLL_PAGE_UP: scroll_pos = std::max(0, scroll_pos - visible_lines); break;
            case Action::SCROLL_PAGE_DOWN: scroll_pos = std::min(max_scroll, scroll_pos + visible_lines); break;
            case Action::GOTO_TOP: scroll_pos = 0; break;
            case Action::GOTO_BOTTOM: scroll_pos = max_scroll; break;
            case Action::GOTO_LINE: if (result.number > 0) scroll_pos = std::min(result.number - 1, max_scroll); break;

            case Action::NEXT_LINK:
                if (!interactive_elements.empty()) {
                    current_element_index = (current_element_index + 1) % interactive_elements.size();
                    scroll_to_element(current_element_index);
                }
                break;

            case Action::PREV_LINK:
                if (!interactive_elements.empty()) {
                    current_element_index = (current_element_index - 1 + interactive_elements.size()) % interactive_elements.size();
                    scroll_to_element(current_element_index);
                }
                break;

            case Action::FOLLOW_LINK:
                activate_element(current_element_index);
                break;

            case Action::GO_BACK:
                if (history_pos > 0) { history_pos--; load_page(history[history_pos]); }
                break;
            case Action::GO_FORWARD:
                if (history_pos < static_cast<int>(history.size()) - 1) { history_pos++; load_page(history[history_pos]); }
                break;
            case Action::OPEN_URL: if (!result.text.empty()) load_page(result.text); break;
            case Action::REFRESH: if (!current_url.empty()) load_page(current_url); break;
            
            case Action::SEARCH_FORWARD:
                search_term = result.text;
                search_results.clear();
                for (size_t i = 0; i < rendered_lines.size(); ++i) {
                    if (rendered_lines[i].text.find(search_term) != std::string::npos) search_results.push_back(i);
                }
                if (!search_results.empty()) {
                    scroll_pos = search_results[0];
                    status_message = "Found " + std::to_string(search_results.size()) + " matches";
                } else status_message = "Pattern not found";
                break;

            case Action::SEARCH_NEXT:
                if (!search_results.empty()) {
                    auto it = std::upper_bound(search_results.begin(), search_results.end(), scroll_pos);
                    scroll_pos = (it != search_results.end()) ? *it : search_results[0];
                }
                break;
            case Action::SEARCH_PREV:
                if (!search_results.empty()) {
                    auto it = std::lower_bound(search_results.begin(), search_results.end(), scroll_pos);
                    scroll_pos = (it != search_results.begin()) ? *(--it) : search_results.back();
                }
                break;
            
            case Action::HELP: show_help(); break;
            case Action::QUIT: break; // Handled in browser.run
            default: break;
        }
    }

    void scroll_to_element(int index) {
        if (index < 0 || index >= static_cast<int>(interactive_elements.size())) return;
        
        int line_idx = interactive_elements[index].line_index;
        int visible_lines = screen_height - 2;
        
        if (line_idx < scroll_pos || line_idx >= scroll_pos + visible_lines) {
            scroll_pos = std::max(0, line_idx - visible_lines / 2);
        }
    }

    void show_help() {
        // Updated help text would go here
        std::ostringstream help_html;
        help_html << "<html><body><h1>Help</h1><p>Use Tab to navigate links and form fields.</p><p>Enter to activate/edit.</p></body></html>";
        current_tree = html_parser.parse_tree(help_html.str(), "help://");
        rendered_lines = renderer.render_tree(current_tree, screen_width);
        build_interactive_list();
        scroll_pos = 0;
        current_element_index = -1;
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

    if (!initial_url.empty()) load_url(initial_url);
    else pImpl->show_help();

    bool running = true;
    while (running) {
        pImpl->draw_screen();
        refresh();

        int ch = getch();
        if (ch == ERR) { napms(50); continue; }

        if (ch == KEY_MOUSE) {
            MEVENT event;
            if (getmouse(&event) == OK) pImpl->handle_mouse(event);
            continue;
        }

        auto result = pImpl->input_handler.handle_key(ch);
        if (result.action == Action::QUIT) running = false;
        else if (result.action != Action::NONE) pImpl->handle_action(result);
    }

    pImpl->cleanup_screen();
}

bool Browser::load_url(const std::string& url) {
    return pImpl->load_page(url);
}

std::string Browser::get_current_url() const {
    return pImpl->current_url;
}