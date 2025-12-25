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

struct RenderConfig {
    int max_width = 80;
    int margin_left = 0;
    bool center_content = false;  // 改为false：全宽渲染
    int paragraph_spacing = 1;
    bool show_link_indicators = false;  // Set to false to show inline links by default
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
