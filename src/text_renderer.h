#pragma once

#include "html_parser.h"
#include <string>
#include <vector>
#include <memory>
#include <curses.h>

// Forward declarations
struct DocumentTree;
struct DomNode;

struct InteractiveRange {
    size_t start;
    size_t end;
    int link_index = -1;
    int field_index = -1;
};

struct RenderedLine {
    std::string text;
    int color_pair;
    bool is_bold;
    bool is_link;
    int link_index;
    std::vector<InteractiveRange> interactive_ranges;
};

// Unicode装饰字符
namespace UnicodeChars {
    // 框线字符 (Box Drawing)
    constexpr const char* DBL_HORIZONTAL = "═";
    constexpr const char* DBL_VERTICAL = "║";
    constexpr const char* DBL_TOP_LEFT = "╔";
    constexpr const char* DBL_TOP_RIGHT = "╗";
    constexpr const char* DBL_BOTTOM_LEFT = "╚";
    constexpr const char* DBL_BOTTOM_RIGHT = "╝";

    constexpr const char* SGL_HORIZONTAL = "─";
    constexpr const char* SGL_VERTICAL = "│";
    constexpr const char* SGL_TOP_LEFT = "┌";
    constexpr const char* SGL_TOP_RIGHT = "┐";
    constexpr const char* SGL_BOTTOM_LEFT = "└";
    constexpr const char* SGL_BOTTOM_RIGHT = "┘";
    constexpr const char* SGL_CROSS = "┼";
    constexpr const char* SGL_T_DOWN = "┬";
    constexpr const char* SGL_T_UP = "┴";
    constexpr const char* SGL_T_RIGHT = "├";
    constexpr const char* SGL_T_LEFT = "┤";

    constexpr const char* HEAVY_HORIZONTAL = "━";
    constexpr const char* HEAVY_VERTICAL = "┃";

    // 列表符号
    constexpr const char* BULLET = "•";
    constexpr const char* CIRCLE = "◦";
    constexpr const char* SQUARE = "▪";
    constexpr const char* TRIANGLE = "‣";

    // 装饰符号
    constexpr const char* SECTION = "§";
    constexpr const char* PARAGRAPH = "¶";
    constexpr const char* ARROW_RIGHT = "→";
    constexpr const char* ELLIPSIS = "…";
}

struct RenderConfig {
    // 布局设置
    int max_width = 80;              // 最大内容宽度
    int margin_left = 0;             // 左边距
    bool center_content = false;     // 内容居中
    int paragraph_spacing = 1;       // 段落间距

    // 响应式宽度设置
    bool responsive_width = true;    // 启用响应式宽度
    int min_width = 60;              // 最小内容宽度
    int max_content_width = 100;     // 最大内容宽度
    int small_screen_threshold = 80; // 小屏阈值
    int large_screen_threshold = 120;// 大屏阈值

    // 链接设置
    bool show_link_indicators = false; // 不显示[N]编号
    bool inline_links = true;          // 内联链接（仅颜色）

    // 视觉样式
    bool use_unicode_boxes = true;   // 使用Unicode框线
    bool use_fancy_bullets = true;   // 使用精美列表符号
    bool show_decorative_lines = true; // 显示装饰线

    // 标题样式
    bool h1_use_double_border = true; // H1使用双线框
    bool h2_use_single_border = true; // H2使用单线框
    bool h3_use_underline = true;     // H3使用下划线
};

// 渲染上下文
struct RenderContext {
    int screen_width;        // 终端宽度
    int current_indent;      // 当前缩进级别
    int nesting_level;       // 列表嵌套层级
    int color_pair;          // 当前颜色
    bool is_bold;            // 是否加粗
};

class TextRenderer {
public:
    TextRenderer();
    ~TextRenderer();

    // 新接口：从DOM树渲染
    std::vector<RenderedLine> render_tree(const DocumentTree& tree, int screen_width);

    // 旧接口：向后兼容
    std::vector<RenderedLine> render(const ParsedDocument& doc, int screen_width);

    void set_config(const RenderConfig& config);
    RenderConfig get_config() const;

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};

enum ColorScheme {
    COLOR_NORMAL = 1,
    COLOR_HEADING1,
    COLOR_HEADING2,
    COLOR_HEADING3,
    COLOR_LINK,
    COLOR_LINK_ACTIVE,
    COLOR_STATUS_BAR,
    COLOR_URL_BAR,
    COLOR_SEARCH_HIGHLIGHT,
    COLOR_DIM
};

void init_color_scheme();
