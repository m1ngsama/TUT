#pragma once

#include <string>
#include <functional>
#include <memory>

enum class InputMode {
    NORMAL,
    COMMAND,
    SEARCH,
    LINK,
    LINK_HINTS     // Vimium-style 'f' mode
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
    GOTO_LINK,        // Jump to specific link by number
    FOLLOW_LINK_NUM,  // Follow specific link by number (f command)
    SHOW_LINK_HINTS,  // Activate link hints mode ('f')
    FOLLOW_LINK_HINT, // Follow link by hint letters
    GO_BACK,
    GO_FORWARD,
    OPEN_URL,
    REFRESH,
    QUIT,
    HELP,
    SET_MARK,               // Set a mark (m + letter)
    GOTO_MARK,              // Jump to mark (' + letter)
    ADD_BOOKMARK,           // Add current page to bookmarks (B)
    REMOVE_BOOKMARK,        // Remove current page from bookmarks (D)
    SHOW_BOOKMARKS,         // Show bookmarks page (:bookmarks)
    SHOW_HISTORY            // Show history page (:history)
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
