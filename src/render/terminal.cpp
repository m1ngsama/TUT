#include "terminal.h"
#include <ncurses.h>
#include <cstdlib>
#include <cstdio>
#include <cstring>
#include <locale.h>

namespace tut {

// ==================== Terminal::Impl ====================

class Terminal::Impl {
public:
    Impl()
        : initialized_(false)
        , has_true_color_(false)
        , has_mouse_(false)
        , has_unicode_(false)
        , has_italic_(false)
        , width_(0)
        , height_(0)
        , mouse_enabled_(false)
    {}

    ~Impl() {
        if (initialized_) {
            cleanup();
        }
    }

    bool init() {
        if (initialized_) {
            return true;
        }

        // 设置locale以支持UTF-8
        setlocale(LC_ALL, "");

        // 初始化ncurses
        initscr();
        if (stdscr == nullptr) {
            return false;
        }

        // 基础设置
        raw();              // 禁用行缓冲
        noecho();           // 不回显输入
        keypad(stdscr, TRUE); // 启用功能键
        nodelay(stdscr, TRUE); // 非阻塞输入（默认）

        // 检测终端能力
        detect_capabilities();

        // 获取屏幕尺寸
        getmaxyx(stdscr, height_, width_);

        // 隐藏光标（默认）
        curs_set(0);

        // 启用鼠标支持
        if (has_mouse_) {
            enable_mouse(true);
        }

        // 使用替代屏幕缓冲区
        use_alternate_screen(true);

        initialized_ = true;
        return true;
    }

    void cleanup() {
        if (!initialized_) {
            return;
        }

        // 恢复光标
        curs_set(1);

        // 禁用鼠标
        if (mouse_enabled_) {
            enable_mouse(false);
        }

        // 退出替代屏幕
        use_alternate_screen(false);

        // 清理ncurses
        endwin();

        initialized_ = false;
    }

    void detect_capabilities() {
        // 检测True Color支持
        const char* colorterm = std::getenv("COLORTERM");
        has_true_color_ = (colorterm != nullptr &&
                          (std::strcmp(colorterm, "truecolor") == 0 ||
                           std::strcmp(colorterm, "24bit") == 0));

        // 检测鼠标支持
        has_mouse_ = has_mouse();

        // 检测Unicode支持（通过locale）
        const char* lang = std::getenv("LANG");
        has_unicode_ = (lang != nullptr &&
                       (std::strstr(lang, "UTF-8") != nullptr ||
                        std::strstr(lang, "utf8") != nullptr));

        // 检测斜体支持（大多数现代终端支持）
        const char* term = std::getenv("TERM");
        has_italic_ = (term != nullptr &&
                      (std::strstr(term, "xterm") != nullptr ||
                       std::strstr(term, "screen") != nullptr ||
                       std::strstr(term, "tmux") != nullptr ||
                       std::strstr(term, "kitty") != nullptr ||
                       std::strstr(term, "alacritty") != nullptr));
    }

    void get_size(int& width, int& height) {
        // 每次调用时获取最新尺寸，以支持窗口大小调整
        getmaxyx(stdscr, height_, width_);
        width = width_;
        height = height_;
    }

    void clear() {
        ::clear();
    }

    void refresh() {
        ::refresh();
    }

    // ==================== True Color ====================

    void set_foreground(uint32_t rgb) {
        if (has_true_color_) {
            // ANSI escape: ESC[38;2;R;G;Bm
            int r = (rgb >> 16) & 0xFF;
            int g = (rgb >> 8) & 0xFF;
            int b = rgb & 0xFF;
            std::printf("\033[38;2;%d;%d;%dm", r, g, b);
            std::fflush(stdout);
        } else {
            // 降级到基础色（简化映射）
            // 这里可以实现256色或8色的映射
            // 暂时使用默认色
        }
    }

    void set_background(uint32_t rgb) {
        if (has_true_color_) {
            // ANSI escape: ESC[48;2;R;G;Bm
            int r = (rgb >> 16) & 0xFF;
            int g = (rgb >> 8) & 0xFF;
            int b = rgb & 0xFF;
            std::printf("\033[48;2;%d;%d;%dm", r, g, b);
            std::fflush(stdout);
        }
    }

    void reset_colors() {
        // ESC[39m 重置前景色, ESC[49m 重置背景色
        std::printf("\033[39m\033[49m");
        std::fflush(stdout);
    }

    // ==================== 文本属性 ====================

    void set_bold(bool enabled) {
        if (enabled) {
            std::printf("\033[1m");  // ESC[1m
        } else {
            std::printf("\033[22m"); // ESC[22m (normal intensity)
        }
        std::fflush(stdout);
    }

    void set_italic(bool enabled) {
        if (!has_italic_) return;

        if (enabled) {
            std::printf("\033[3m");  // ESC[3m
        } else {
            std::printf("\033[23m"); // ESC[23m
        }
        std::fflush(stdout);
    }

    void set_underline(bool enabled) {
        if (enabled) {
            std::printf("\033[4m");  // ESC[4m
        } else {
            std::printf("\033[24m"); // ESC[24m
        }
        std::fflush(stdout);
    }

    void set_reverse(bool enabled) {
        if (enabled) {
            std::printf("\033[7m");  // ESC[7m
        } else {
            std::printf("\033[27m"); // ESC[27m
        }
        std::fflush(stdout);
    }

    void set_dim(bool enabled) {
        if (enabled) {
            std::printf("\033[2m");  // ESC[2m
        } else {
            std::printf("\033[22m"); // ESC[22m
        }
        std::fflush(stdout);
    }

    void reset_attributes() {
        std::printf("\033[0m");  // ESC[0m (reset all)
        std::fflush(stdout);
    }

    // ==================== 光标控制 ====================

    void move_cursor(int x, int y) {
        move(y, x);  // ncurses使用 (y, x) 顺序
    }

    void hide_cursor() {
        curs_set(0);
    }

    void show_cursor() {
        curs_set(1);
    }

    // ==================== 文本输出 ====================

    void print(const std::string& text) {
        // 直接输出到stdout（配合ANSI escape sequences）
        std::printf("%s", text.c_str());
        std::fflush(stdout);
    }

    void print_at(int x, int y, const std::string& text) {
        move_cursor(x, y);
        print(text);
    }

    // ==================== 输入处理 ====================

    int get_key(int timeout_ms) {
        if (timeout_ms == -1) {
            // 阻塞等待
            nodelay(stdscr, FALSE);
            int ch = getch();
            nodelay(stdscr, TRUE);
            return ch;
        } else if (timeout_ms == 0) {
            // 非阻塞
            return getch();
        } else {
            // 超时等待
            timeout(timeout_ms);
            int ch = getch();
            nodelay(stdscr, TRUE);
            return ch;
        }
    }

    bool get_mouse_event(MouseEvent& event) {
        if (!mouse_enabled_) {
            return false;
        }

        MEVENT mevent;
        int ch = getch();

        if (ch == KEY_MOUSE) {
            if (getmouse(&mevent) == OK) {
                event.x = mevent.x;
                event.y = mevent.y;

                // 解析鼠标事件类型
                if (mevent.bstate & BUTTON1_CLICKED) {
                    event.type = MouseEvent::Type::CLICK;
                    event.button = 0;
                    return true;
                } else if (mevent.bstate & BUTTON2_CLICKED) {
                    event.type = MouseEvent::Type::CLICK;
                    event.button = 1;
                    return true;
                } else if (mevent.bstate & BUTTON3_CLICKED) {
                    event.type = MouseEvent::Type::CLICK;
                    event.button = 2;
                    return true;
                }
#ifdef BUTTON4_PRESSED
                else if (mevent.bstate & BUTTON4_PRESSED) {
                    event.type = MouseEvent::Type::SCROLL_UP;
                    return true;
                }
#endif
#ifdef BUTTON5_PRESSED
                else if (mevent.bstate & BUTTON5_PRESSED) {
                    event.type = MouseEvent::Type::SCROLL_DOWN;
                    return true;
                }
#endif
            }
        }

        return false;
    }

    // ==================== 终端能力 ====================

    bool supports_true_color() const { return has_true_color_; }
    bool supports_mouse() const { return has_mouse_; }
    bool supports_unicode() const { return has_unicode_; }
    bool supports_italic() const { return has_italic_; }

    // ==================== 高级功能 ====================

    void enable_mouse(bool enabled) {
        if (enabled) {
            // 启用所有鼠标事件
            mousemask(ALL_MOUSE_EVENTS | REPORT_MOUSE_POSITION, nullptr);
            // 发送启用鼠标跟踪的ANSI序列
            std::printf("\033[?1003h"); // 启用所有鼠标事件
            std::fflush(stdout);
            mouse_enabled_ = true;
        } else {
            mousemask(0, nullptr);
            std::printf("\033[?1003l"); // 禁用鼠标跟踪
            std::fflush(stdout);
            mouse_enabled_ = false;
        }
    }

    void use_alternate_screen(bool enabled) {
        if (enabled) {
            std::printf("\033[?1049h"); // 进入替代屏幕
        } else {
            std::printf("\033[?1049l"); // 退出替代屏幕
        }
        std::fflush(stdout);
    }

private:
    bool initialized_;
    bool has_true_color_;
    bool has_mouse_;
    bool has_unicode_;
    bool has_italic_;
    int width_;
    int height_;
    bool mouse_enabled_;
};

// ==================== Terminal 公共接口 ====================

Terminal::Terminal() : pImpl(std::make_unique<Impl>()) {}
Terminal::~Terminal() = default;

bool Terminal::init() { return pImpl->init(); }
void Terminal::cleanup() { pImpl->cleanup(); }

void Terminal::get_size(int& width, int& height) {
    pImpl->get_size(width, height);
}
void Terminal::clear() { pImpl->clear(); }
void Terminal::refresh() { pImpl->refresh(); }

void Terminal::set_foreground(uint32_t rgb) { pImpl->set_foreground(rgb); }
void Terminal::set_background(uint32_t rgb) { pImpl->set_background(rgb); }
void Terminal::reset_colors() { pImpl->reset_colors(); }

void Terminal::set_bold(bool enabled) { pImpl->set_bold(enabled); }
void Terminal::set_italic(bool enabled) { pImpl->set_italic(enabled); }
void Terminal::set_underline(bool enabled) { pImpl->set_underline(enabled); }
void Terminal::set_reverse(bool enabled) { pImpl->set_reverse(enabled); }
void Terminal::set_dim(bool enabled) { pImpl->set_dim(enabled); }
void Terminal::reset_attributes() { pImpl->reset_attributes(); }

void Terminal::move_cursor(int x, int y) { pImpl->move_cursor(x, y); }
void Terminal::hide_cursor() { pImpl->hide_cursor(); }
void Terminal::show_cursor() { pImpl->show_cursor(); }

void Terminal::print(const std::string& text) { pImpl->print(text); }
void Terminal::print_at(int x, int y, const std::string& text) {
    pImpl->print_at(x, y, text);
}

int Terminal::get_key(int timeout_ms) { return pImpl->get_key(timeout_ms); }
bool Terminal::get_mouse_event(MouseEvent& event) {
    return pImpl->get_mouse_event(event);
}

bool Terminal::supports_true_color() const { return pImpl->supports_true_color(); }
bool Terminal::supports_mouse() const { return pImpl->supports_mouse(); }
bool Terminal::supports_unicode() const { return pImpl->supports_unicode(); }
bool Terminal::supports_italic() const { return pImpl->supports_italic(); }

void Terminal::enable_mouse(bool enabled) { pImpl->enable_mouse(enabled); }
void Terminal::use_alternate_screen(bool enabled) {
    pImpl->use_alternate_screen(enabled);
}

} // namespace tut
