#pragma once

#include "html_parser.h"
#include <string>
#include <vector>
#include <memory>
#include <curses.h>

struct RenderedLine {
    std::string text;
    int color_pair;
    bool is_bold;
    bool is_link;
    int link_index;
    std::vector<std::pair<size_t, size_t>> link_ranges;  // (start, end) positions of links in this line
};

struct RenderConfig {
    int max_width = 80;
    int margin_left = 0;
    bool center_content = true;
    int paragraph_spacing = 1;
    bool show_link_indicators = false;  // Set to false to show inline links by default
};

class TextRenderer {
public:
    TextRenderer();
    ~TextRenderer();

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
