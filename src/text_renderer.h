#pragma once

#include "html_parser.h"
#include <string>
#include <vector>
#include <memory>
#include <curses.h>

// 渲染后的行信息
struct RenderedLine {
    std::string text;
    int color_pair;
    bool is_bold;
    bool is_link;
    int link_index; // 如果是链接，对应的链接索引
};

// 渲染配置
struct RenderConfig {
    int max_width = 80;        // 内容最大宽度
    int margin_left = 0;       // 左边距（居中时自动计算）
    bool center_content = true; // 是否居中内容
    int paragraph_spacing = 1;  // 段落间距
    bool show_link_indicators = true; // 是否显示链接指示器
};

class TextRenderer {
public:
    TextRenderer();
    ~TextRenderer();

    // 渲染文档到行数组
    std::vector<RenderedLine> render(const ParsedDocument& doc, int screen_width);

    // 设置渲染配置
    void set_config(const RenderConfig& config);

    // 获取当前配置
    RenderConfig get_config() const;

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};

// 颜色定义
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

// 初始化颜色方案
void init_color_scheme();
