#include "input_handler.h"
#include <curses.h>
#include <cctype>
#include <sstream>

class InputHandler::Impl {
public:
    InputMode mode = InputMode::NORMAL;
    std::string buffer;
    std::string count_buffer;
    std::function<void(const std::string&)> status_callback;

    void set_status(const std::string& msg) {
        if (status_callback) {
            status_callback(msg);
        }
    }

    InputResult process_normal_mode(int ch) {
        InputResult result;
        result.action = Action::NONE;
        result.number = 0;
        result.has_count = false;
        result.count = 1;

        // Handle multi-char commands like 'gg', 'gt', 'gT', 'm', '
        if (!buffer.empty()) {
            if (buffer == "g") {
                if (ch == 't') {
                    result.action = Action::NEXT_TAB;
                    buffer.clear();
                    count_buffer.clear();
                    return result;
                } else if (ch == 'T') {
                    result.action = Action::PREV_TAB;
                    buffer.clear();
                    count_buffer.clear();
                    return result;
                }
            } else if (buffer == "m") {
                // Set mark with letter
                if (std::isalpha(ch)) {
                    result.action = Action::SET_MARK;
                    result.text = std::string(1, static_cast<char>(ch));
                    buffer.clear();
                    count_buffer.clear();
                    return result;
                }
                buffer.clear();
                count_buffer.clear();
                return result;
            } else if (buffer == "'") {
                // Jump to mark
                if (std::isalpha(ch)) {
                    result.action = Action::GOTO_MARK;
                    result.text = std::string(1, static_cast<char>(ch));
                    buffer.clear();
                    count_buffer.clear();
                    return result;
                }
                buffer.clear();
                count_buffer.clear();
                return result;
            }
        }

        // Handle digit input for count
        if (std::isdigit(ch) && (ch != '0' || !count_buffer.empty())) {
            count_buffer += static_cast<char>(ch);
            return result;
        }

        if (!count_buffer.empty()) {
            result.has_count = true;
            result.count = std::stoi(count_buffer);
        }

        switch (ch) {
            case 'j':
            case KEY_DOWN:
                result.action = Action::SCROLL_DOWN;
                count_buffer.clear();
                break;
            case 'k':
            case KEY_UP:
                result.action = Action::SCROLL_UP;
                count_buffer.clear();
                break;
            case 'h':
            case KEY_LEFT:
                result.action = Action::GO_BACK;
                count_buffer.clear();
                break;
            case 'l':
            case KEY_RIGHT:
                result.action = Action::GO_FORWARD;
                count_buffer.clear();
                break;
            case 4:
            case ' ':
                result.action = Action::SCROLL_PAGE_DOWN;
                count_buffer.clear();
                break;
            case 21:
            case 'b':
                result.action = Action::SCROLL_PAGE_UP;
                count_buffer.clear();
                break;
            case 'g':
                buffer += 'g';
                if (buffer == "gg") {
                    result.action = Action::GOTO_TOP;
                    buffer.clear();
                }
                count_buffer.clear();
                break;
            case 'G':
                if (result.has_count) {
                    result.action = Action::GOTO_LINE;
                    result.number = result.count;
                } else {
                    result.action = Action::GOTO_BOTTOM;
                }
                count_buffer.clear();
                break;
            case '/':
                mode = InputMode::SEARCH;
                buffer = "/";
                count_buffer.clear();
                break;
            case 'n':
                result.action = Action::SEARCH_NEXT;
                count_buffer.clear();
                break;
            case 'N':
                result.action = Action::SEARCH_PREV;
                count_buffer.clear();
                break;
            case '\t':
                result.action = Action::NEXT_LINK;
                count_buffer.clear();
                break;
            case KEY_BTAB:
            case 'T':
                result.action = Action::PREV_LINK;
                count_buffer.clear();
                break;
            case '\n':
            case '\r':
                // If count buffer has a number, jump to that link
                if (result.has_count) {
                    result.action = Action::GOTO_LINK;
                    result.number = result.count;
                } else {
                    result.action = Action::FOLLOW_LINK;
                }
                count_buffer.clear();
                break;
            case 'f':
                // 'f' command: vimium-style link hints
                result.action = Action::SHOW_LINK_HINTS;
                mode = InputMode::LINK_HINTS;
                buffer.clear();
                count_buffer.clear();
                break;
            case 'v':
                // Enter visual mode
                result.action = Action::ENTER_VISUAL_MODE;
                mode = InputMode::VISUAL;
                count_buffer.clear();
                break;
            case 'V':
                // Enter visual line mode
                result.action = Action::ENTER_VISUAL_LINE_MODE;
                mode = InputMode::VISUAL_LINE;
                count_buffer.clear();
                break;
            case 'm':
                // Set mark (wait for next char)
                buffer = "m";
                count_buffer.clear();
                break;
            case '\'':
                // Jump to mark (wait for next char)
                buffer = "'";
                count_buffer.clear();
                break;
            case ':':
                mode = InputMode::COMMAND;
                buffer = ":";
                break;
            case 'r':
                result.action = Action::REFRESH;
                break;
            case 'q':
                result.action = Action::QUIT;
                break;
            case '?':
                result.action = Action::HELP;
                break;
            default:
                buffer.clear();
                break;
        }

        return result;
    }

    InputResult process_command_mode(int ch) {
        InputResult result;
        result.action = Action::NONE;

        if (ch == '\n' || ch == '\r') {
            std::string command = buffer.substr(1);

            if (command == "q" || command == "quit") {
                result.action = Action::QUIT;
            } else if (command == "h" || command == "help") {
                result.action = Action::HELP;
            } else if (command == "r" || command == "refresh") {
                result.action = Action::REFRESH;
            } else if (command.rfind("o ", 0) == 0 || command.rfind("open ", 0) == 0) {
                size_t space_pos = command.find(' ');
                if (space_pos != std::string::npos) {
                    result.action = Action::OPEN_URL;
                    result.text = command.substr(space_pos + 1);
                }
            } else if (!command.empty() && std::isdigit(command[0])) {
                try {
                    result.action = Action::GOTO_LINE;
                    result.number = std::stoi(command);
                } catch (...) {
                    set_status("Invalid line number");
                }
            }

            mode = InputMode::NORMAL;
            buffer.clear();
        } else if (ch == 27) {
            mode = InputMode::NORMAL;
            buffer.clear();
        } else if (ch == KEY_BACKSPACE || ch == 127 || ch == 8) {
            if (buffer.length() > 1) {
                buffer.pop_back();
            } else {
                mode = InputMode::NORMAL;
                buffer.clear();
            }
        } else if (std::isprint(ch)) {
            buffer += static_cast<char>(ch);
        }

        return result;
    }

    InputResult process_search_mode(int ch) {
        InputResult result;
        result.action = Action::NONE;

        if (ch == '\n' || ch == '\r') {
            if (buffer.length() > 1) {
                result.action = Action::SEARCH_FORWARD;
                result.text = buffer.substr(1);
            }
            mode = InputMode::NORMAL;
            buffer.clear();
        } else if (ch == 27) {
            mode = InputMode::NORMAL;
            buffer.clear();
        } else if (ch == KEY_BACKSPACE || ch == 127 || ch == 8) {
            if (buffer.length() > 1) {
                buffer.pop_back();
            } else {
                mode = InputMode::NORMAL;
                buffer.clear();
            }
        } else if (std::isprint(ch)) {
            buffer += static_cast<char>(ch);
        }

        return result;
    }

    InputResult process_link_mode(int ch) {
        InputResult result;
        result.action = Action::NONE;

        if (std::isdigit(ch)) {
            buffer += static_cast<char>(ch);
        } else if (ch == '\n' || ch == '\r') {
            // Follow the link number entered
            if (buffer.length() > 1) {
                try {
                    int link_num = std::stoi(buffer.substr(1));
                    result.action = Action::FOLLOW_LINK_NUM;
                    result.number = link_num;
                } catch (...) {
                    set_status("Invalid link number");
                }
            }
            mode = InputMode::NORMAL;
            buffer.clear();
        } else if (ch == 27) {
            // ESC cancels
            mode = InputMode::NORMAL;
            buffer.clear();
        } else if (ch == KEY_BACKSPACE || ch == 127 || ch == 8) {
            if (buffer.length() > 1) {
                buffer.pop_back();
            } else {
                mode = InputMode::NORMAL;
                buffer.clear();
            }
        }

        return result;
    }

    InputResult process_link_hints_mode(int ch) {
        InputResult result;
        result.action = Action::NONE;

        if (ch == 27) {
            // ESC cancels link hints mode
            mode = InputMode::NORMAL;
            buffer.clear();
            return result;
        } else if (ch == KEY_BACKSPACE || ch == 127 || ch == 8) {
            // Backspace removes last character
            if (!buffer.empty()) {
                buffer.pop_back();
            } else {
                mode = InputMode::NORMAL;
            }
            return result;
        } else if (std::isalpha(ch)) {
            // Add character to buffer
            buffer += std::tolower(static_cast<char>(ch));

            // Try to match link hint
            result.action = Action::FOLLOW_LINK_HINT;
            result.text = buffer;

            // Mode will be reset by browser if link is followed
            return result;
        }

        return result;
    }

    InputResult process_visual_mode(int ch) {
        InputResult result;
        result.action = Action::NONE;

        if (ch == 27 || ch == 'v') {
            // ESC or 'v' exits visual mode
            mode = InputMode::NORMAL;
            return result;
        } else if (ch == 'y') {
            // Yank (copy) selected text
            result.action = Action::YANK;
            mode = InputMode::NORMAL;
            return result;
        }

        // Pass through navigation commands
        switch (ch) {
            case 'j':
            case KEY_DOWN:
                result.action = Action::SCROLL_DOWN;
                break;
            case 'k':
            case KEY_UP:
                result.action = Action::SCROLL_UP;
                break;
            case 'h':
            case KEY_LEFT:
                // In visual mode, h/l could extend selection
                break;
            case 'l':
            case KEY_RIGHT:
                break;
        }

        return result;
    }
};

InputHandler::InputHandler() : pImpl(std::make_unique<Impl>()) {}

InputHandler::~InputHandler() = default;

InputResult InputHandler::handle_key(int ch) {
    switch (pImpl->mode) {
        case InputMode::NORMAL:
            return pImpl->process_normal_mode(ch);
        case InputMode::COMMAND:
            return pImpl->process_command_mode(ch);
        case InputMode::SEARCH:
            return pImpl->process_search_mode(ch);
        case InputMode::LINK:
            return pImpl->process_link_mode(ch);
        case InputMode::LINK_HINTS:
            return pImpl->process_link_hints_mode(ch);
        case InputMode::VISUAL:
        case InputMode::VISUAL_LINE:
            return pImpl->process_visual_mode(ch);
        default:
            break;
    }

    InputResult result;
    result.action = Action::NONE;
    return result;
}

InputMode InputHandler::get_mode() const {
    return pImpl->mode;
}

std::string InputHandler::get_buffer() const {
    return pImpl->buffer;
}

void InputHandler::reset() {
    pImpl->mode = InputMode::NORMAL;
    pImpl->buffer.clear();
    pImpl->count_buffer.clear();
}

void InputHandler::set_status_callback(std::function<void(const std::string&)> callback) {
    pImpl->status_callback = callback;
}
