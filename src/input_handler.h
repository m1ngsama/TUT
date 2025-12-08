#pragma once

#include <string>
#include <functional>
#include <memory>

enum class InputMode {
    NORMAL,    // 正常浏览模式
    COMMAND,   // 命令模式 (:)
    SEARCH,    // 搜索模式 (/)
    LINK       // 链接选择模式
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
    std::string text;  // 用于命令、搜索、URL输入
    int number;        // 用于跳转行号、链接编号等
    bool has_count;    // 是否有数字前缀（如 5j）
    int count;         // 数字前缀
};

class InputHandler {
public:
    InputHandler();
    ~InputHandler();

    // 处理单个按键
    InputResult handle_key(int ch);

    // 获取当前模式
    InputMode get_mode() const;

    // 获取当前输入缓冲（用于显示命令行）
    std::string get_buffer() const;

    // 重置状态
    void reset();

    // 设置状态栏消息回调
    void set_status_callback(std::function<void(const std::string&)> callback);

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
