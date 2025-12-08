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

        if (std::isdigit(ch) && (ch != '0' || !count_buffer.empty())) {
            count_buffer += static_cast<char>(ch);
            return result;
        }

        if (!count_buffer.empty()) {
            result.has_count = true;
            result.count = std::stoi(count_buffer);
            count_buffer.clear();
        }

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
                result.action = Action::GO_BACK;
                break;
            case 'l':
            case KEY_RIGHT:
                result.action = Action::GO_FORWARD;
                break;
            case 4:
            case ' ':
                result.action = Action::SCROLL_PAGE_DOWN;
                break;
            case 21:
            case 'b':
                result.action = Action::SCROLL_PAGE_UP;
                break;
            case 'g':
                buffer += 'g';
                if (buffer == "gg") {
                    result.action = Action::GOTO_TOP;
                    buffer.clear();
                }
                break;
            case 'G':
                if (result.has_count) {
                    result.action = Action::GOTO_LINE;
                    result.number = result.count;
                } else {
                    result.action = Action::GOTO_BOTTOM;
                }
                break;
            case '/':
                mode = InputMode::SEARCH;
                buffer = "/";
                break;
            case 'n':
                result.action = Action::SEARCH_NEXT;
                break;
            case 'N':
                result.action = Action::SEARCH_PREV;
                break;
            case '\t':
                result.action = Action::NEXT_LINK;
                break;
            case KEY_BTAB:
            case 'T':
                result.action = Action::PREV_LINK;
                break;
            case '\n':
            case '\r':
                result.action = Action::FOLLOW_LINK;
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
