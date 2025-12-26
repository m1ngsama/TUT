#pragma once

namespace tut {

/**
 * Unicode装饰字符
 *
 * 用于绘制边框、列表符号等装饰元素
 */
namespace chars {

// ==================== 框线字符 (Box Drawing) ====================

// 双线框
constexpr const char* DBL_HORIZONTAL    = "═";
constexpr const char* DBL_VERTICAL      = "║";
constexpr const char* DBL_TOP_LEFT      = "╔";
constexpr const char* DBL_TOP_RIGHT     = "╗";
constexpr const char* DBL_BOTTOM_LEFT   = "╚";
constexpr const char* DBL_BOTTOM_RIGHT  = "╝";
constexpr const char* DBL_T_DOWN        = "╦";
constexpr const char* DBL_T_UP          = "╩";
constexpr const char* DBL_T_RIGHT       = "╠";
constexpr const char* DBL_T_LEFT        = "╣";
constexpr const char* DBL_CROSS         = "╬";

// 单线框
constexpr const char* SGL_HORIZONTAL    = "─";
constexpr const char* SGL_VERTICAL      = "│";
constexpr const char* SGL_TOP_LEFT      = "┌";
constexpr const char* SGL_TOP_RIGHT     = "┐";
constexpr const char* SGL_BOTTOM_LEFT   = "└";
constexpr const char* SGL_BOTTOM_RIGHT  = "┘";
constexpr const char* SGL_T_DOWN        = "┬";
constexpr const char* SGL_T_UP          = "┴";
constexpr const char* SGL_T_RIGHT       = "├";
constexpr const char* SGL_T_LEFT        = "┤";
constexpr const char* SGL_CROSS         = "┼";

// 粗线框
constexpr const char* HEAVY_HORIZONTAL  = "━";
constexpr const char* HEAVY_VERTICAL    = "┃";
constexpr const char* HEAVY_TOP_LEFT    = "┏";
constexpr const char* HEAVY_TOP_RIGHT   = "┓";
constexpr const char* HEAVY_BOTTOM_LEFT = "┗";
constexpr const char* HEAVY_BOTTOM_RIGHT= "┛";

// 圆角框
constexpr const char* ROUND_TOP_LEFT    = "╭";
constexpr const char* ROUND_TOP_RIGHT   = "╮";
constexpr const char* ROUND_BOTTOM_LEFT = "╰";
constexpr const char* ROUND_BOTTOM_RIGHT= "╯";

// ==================== 列表符号 ====================

constexpr const char* BULLET            = "•";
constexpr const char* BULLET_HOLLOW     = "◦";
constexpr const char* BULLET_SQUARE     = "▪";
constexpr const char* CIRCLE            = "◦";
constexpr const char* SQUARE            = "▪";
constexpr const char* TRIANGLE          = "‣";
constexpr const char* DIAMOND           = "◆";
constexpr const char* QUOTE_LEFT        = "│";
constexpr const char* ARROW             = "➤";
constexpr const char* DASH              = "–";
constexpr const char* STAR              = "★";
constexpr const char* CHECK             = "✓";
constexpr const char* CROSS             = "✗";

// ==================== 箭头 ====================

constexpr const char* ARROW_RIGHT       = "→";
constexpr const char* ARROW_LEFT        = "←";
constexpr const char* ARROW_UP          = "↑";
constexpr const char* ARROW_DOWN        = "↓";
constexpr const char* ARROW_DOUBLE_RIGHT= "»";
constexpr const char* ARROW_DOUBLE_LEFT = "«";

// ==================== 装饰符号 ====================

constexpr const char* SECTION           = "§";
constexpr const char* PARAGRAPH         = "¶";
constexpr const char* ELLIPSIS          = "…";
constexpr const char* MIDDOT            = "·";
constexpr const char* DEGREE            = "°";

// ==================== 进度/状态 ====================

constexpr const char* BLOCK_FULL        = "█";
constexpr const char* BLOCK_3_4         = "▓";
constexpr const char* BLOCK_HALF        = "▒";
constexpr const char* BLOCK_1_4         = "░";
constexpr const char* SPINNER[]         = {"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"};
constexpr int SPINNER_FRAMES            = 10;

// ==================== 分隔线样式 ====================

constexpr const char* HR_LIGHT          = "─";
constexpr const char* HR_HEAVY          = "━";
constexpr const char* HR_DOUBLE         = "═";
constexpr const char* HR_DASHED         = "╌";
constexpr const char* HR_DOTTED         = "┄";

} // namespace chars

/**
 * 生成水平分隔线
 */
inline std::string make_horizontal_line(int width, const char* ch = chars::SGL_HORIZONTAL) {
    std::string result;
    for (int i = 0; i < width; i++) {
        result += ch;
    }
    return result;
}

/**
 * 绘制简单边框（单线）
 */
struct BoxChars {
    const char* top_left;
    const char* top_right;
    const char* bottom_left;
    const char* bottom_right;
    const char* horizontal;
    const char* vertical;
};

constexpr BoxChars BOX_SINGLE = {
    chars::SGL_TOP_LEFT, chars::SGL_TOP_RIGHT,
    chars::SGL_BOTTOM_LEFT, chars::SGL_BOTTOM_RIGHT,
    chars::SGL_HORIZONTAL, chars::SGL_VERTICAL
};

constexpr BoxChars BOX_DOUBLE = {
    chars::DBL_TOP_LEFT, chars::DBL_TOP_RIGHT,
    chars::DBL_BOTTOM_LEFT, chars::DBL_BOTTOM_RIGHT,
    chars::DBL_HORIZONTAL, chars::DBL_VERTICAL
};

constexpr BoxChars BOX_HEAVY = {
    chars::HEAVY_TOP_LEFT, chars::HEAVY_TOP_RIGHT,
    chars::HEAVY_BOTTOM_LEFT, chars::HEAVY_BOTTOM_RIGHT,
    chars::HEAVY_HORIZONTAL, chars::HEAVY_VERTICAL
};

constexpr BoxChars BOX_ROUND = {
    chars::ROUND_TOP_LEFT, chars::ROUND_TOP_RIGHT,
    chars::ROUND_BOTTOM_LEFT, chars::ROUND_BOTTOM_RIGHT,
    chars::SGL_HORIZONTAL, chars::SGL_VERTICAL
};

} // namespace tut
