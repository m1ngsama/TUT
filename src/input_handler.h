#pragma once

#include <string>
#include <functional>
#include <memory>

enum class InputMode {
    NORMAL,
    COMMAND,
    SEARCH,
    LINK
};

enum class Action {
    NONE,
    SCROLL_UP,
    SCROLL_DOWN,
    SCROLL_PAGE_UP,
    SCROLL_PAGE_DOWN,
    GOTO_TOP,
    GOTO_BOTTOM,
    GOTO_LINE,
    SEARCH_FORWARD,
    SEARCH_NEXT,
    SEARCH_PREV,
    NEXT_LINK,
    PREV_LINK,
    FOLLOW_LINK,
    GO_BACK,
    GO_FORWARD,
    OPEN_URL,
    REFRESH,
    QUIT,
    HELP
};

struct InputResult {
    Action action;
    std::string text;
    int number;
    bool has_count;
    int count;
};

class InputHandler {
public:
    InputHandler();
    ~InputHandler();

    InputResult handle_key(int ch);
    InputMode get_mode() const;
    std::string get_buffer() const;
    void reset();
    void set_status_callback(std::function<void(const std::string&)> callback);

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
