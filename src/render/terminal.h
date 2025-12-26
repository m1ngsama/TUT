#pragma once

#include <string>
#include <cstdint>
#include <memory>

namespace tut {

// 鼠标事件类型
struct MouseEvent {
    enum class Type {
        CLICK,
        SCROLL_UP,
        SCROLL_DOWN,
        MOVE,
        DRAG
    };

    Type type;
    int x;
    int y;
    int button;  // 0=left, 1=middle, 2=right
};

/**
 * Terminal - 现代终端抽象层
 *
 * 提供True Color (24-bit RGB)支持的终端接口
 * 目标终端: iTerm2, Kitty, Alacritty等现代终端
 *
 * 设计理念:
 * - 优先使用ANSI escape sequences而非ncurses color pairs (突破256色限制)
 * - 检测终端能力并自动降级
 * - 提供清晰的、面向对象的API
 */
class Terminal {
public:
    Terminal();
    ~Terminal();

    // ==================== 初始化与清理 ====================

    /**
     * 初始化终端
     * - 设置原始模式
     * - 检测终端能力
     * - 启用鼠标支持（如果可用）
     * @return 是否成功初始化
     */
    bool init();

    /**
     * 清理并恢复终端状态
     */
    void cleanup();

    // ==================== 屏幕管理 ====================

    /**
     * 获取终端尺寸（每次调用都会获取最新尺寸）
     */
    void get_size(int& width, int& height);

    /**
     * 清空屏幕
     */
    void clear();

    /**
     * 刷新显示（将缓冲区内容显示到屏幕）
     */
    void refresh();

    // ==================== True Color 支持 ====================

    /**
     * 设置前景色 (24-bit RGB)
     * @param rgb RGB颜色值，格式: 0xRRGGBB
     * 示例: 0xE8C48C (暖金色)
     */
    void set_foreground(uint32_t rgb);

    /**
     * 设置背景色 (24-bit RGB)
     * @param rgb RGB颜色值，格式: 0xRRGGBB
     */
    void set_background(uint32_t rgb);

    /**
     * 重置颜色为默认值
     */
    void reset_colors();

    // ==================== 文本属性 ====================

    /**
     * 设置粗体
     */
    void set_bold(bool enabled);

    /**
     * 设置斜体
     */
    void set_italic(bool enabled);

    /**
     * 设置下划线
     */
    void set_underline(bool enabled);

    /**
     * 设置反色显示
     */
    void set_reverse(bool enabled);

    /**
     * 设置暗淡显示
     */
    void set_dim(bool enabled);

    /**
     * 重置所有文本属性
     */
    void reset_attributes();

    // ==================== 光标控制 ====================

    /**
     * 移动光标到指定位置
     * @param x 列位置 (0-based)
     * @param y 行位置 (0-based)
     */
    void move_cursor(int x, int y);

    /**
     * 隐藏光标
     */
    void hide_cursor();

    /**
     * 显示光标
     */
    void show_cursor();

    // ==================== 文本输出 ====================

    /**
     * 在当前光标位置输出文本
     */
    void print(const std::string& text);

    /**
     * 在指定位置输出文本
     */
    void print_at(int x, int y, const std::string& text);

    // ==================== 输入处理 ====================

    /**
     * 获取按键
     * @param timeout_ms 超时时间（毫秒），-1表示阻塞等待
     * @return 按键代码，超时返回-1
     */
    int get_key(int timeout_ms = -1);

    /**
     * 获取鼠标事件
     * @param event 输出参数，存储鼠标事件
     * @return 是否成功获取鼠标事件
     */
    bool get_mouse_event(MouseEvent& event);

    // ==================== 终端能力检测 ====================

    /**
     * 是否支持True Color (24-bit)
     * 检测方法: 环境变量 COLORTERM=truecolor 或 COLORTERM=24bit
     */
    bool supports_true_color() const;

    /**
     * 是否支持鼠标
     */
    bool supports_mouse() const;

    /**
     * 是否支持Unicode
     */
    bool supports_unicode() const;

    /**
     * 是否支持斜体
     */
    bool supports_italic() const;

    // ==================== 高级功能 ====================

    /**
     * 启用/禁用鼠标支持
     */
    void enable_mouse(bool enabled);

    /**
     * 启用/禁用替代屏幕缓冲区
     * (用于全屏应用，退出时恢复原屏幕内容)
     */
    void use_alternate_screen(bool enabled);

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;

    // 禁止拷贝
    Terminal(const Terminal&) = delete;
    Terminal& operator=(const Terminal&) = delete;
};

} // namespace tut
