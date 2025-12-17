#include "text_renderer.h"
#include <sstream>
#include <algorithm>
#include <clocale>
#include <numeric>

// Box-drawing characters for tables
namespace BoxChars {
    constexpr const char* TOP_LEFT = "┌";
    constexpr const char* TOP_RIGHT = "┐";
    constexpr const char* BOTTOM_LEFT = "└";
    constexpr const char* BOTTOM_RIGHT = "┘";
    constexpr const char* HORIZONTAL = "─";
    constexpr const char* VERTICAL = "│";
    constexpr const char* T_DOWN = "┬";
    constexpr const char* T_UP = "┴";
    constexpr const char* T_RIGHT = "├";
    constexpr const char* T_LEFT = "┤";
    constexpr const char* CROSS = "┼";
    constexpr const char* HEAVY_HORIZONTAL = "━";
    constexpr const char* HEAVY_VERTICAL = "┃";
}

class TextRenderer::Impl {
public:
    RenderConfig config;

    struct LinkPosition {
        int link_index;
        size_t start;
        size_t end;
    };

    std::vector<std::string> wrap_text(const std::string& text, int width) {
        std::vector<std::string> lines;
        if (text.empty()) {
            return lines;
        }

        std::istringstream words_stream(text);
        std::string word;
        std::string current_line;

        while (words_stream >> word) {
            if (word.length() > static_cast<size_t>(width)) {
                if (!current_line.empty()) {
                    lines.push_back(current_line);
                    current_line.clear();
                }
                for (size_t i = 0; i < word.length(); i += width) {
                    lines.push_back(word.substr(i, width));
                }
                continue;
            }

            if (current_line.empty()) {
                current_line = word;
            } else if (current_line.length() + 1 + word.length() <= static_cast<size_t>(width)) {
                current_line += " " + word;
            } else {
                lines.push_back(current_line);
                current_line = word;
            }
        }

        if (!current_line.empty()) {
            lines.push_back(current_line);
        }

        if (lines.empty()) {
            lines.push_back("");
        }

        return lines;
    }

    // Wrap text with links, tracking link positions and adding link numbers
    std::vector<std::pair<std::string, std::vector<LinkPosition>>>
    wrap_text_with_links(const std::string& original_text, int width,
                        const std::vector<InlineLink>& inline_links) {
        std::vector<std::pair<std::string, std::vector<LinkPosition>>> result;

        // If no links, use simple wrapping
        if (inline_links.empty()) {
            auto wrapped = wrap_text(original_text, width);
            for (const auto& line : wrapped) {
                result.push_back({line, {}});
            }
            return result;
        }

        // Build modified text with link numbers inserted
        std::string text;
        std::vector<InlineLink> modified_links;
        size_t text_pos = 0;

        for (const auto& link : inline_links) {
            // Add text before link
            if (link.start_pos > text_pos) {
                text += original_text.substr(text_pos, link.start_pos - text_pos);
            }

            // Add link text with number indicator
            size_t link_start_in_modified = text.length();
            std::string link_text = original_text.substr(link.start_pos, link.end_pos - link.start_pos);
            std::string link_indicator = "[" + std::to_string(link.link_index) + "]";
            text += link_text + link_indicator;

            // Store modified link position (including the indicator)
            InlineLink mod_link = link;
            mod_link.start_pos = link_start_in_modified;
            mod_link.end_pos = text.length();
            modified_links.push_back(mod_link);

            text_pos = link.end_pos;
        }

        // Add remaining text after last link
        if (text_pos < original_text.length()) {
            text += original_text.substr(text_pos);
        }

        // Split text into words
        std::vector<std::string> words;
        std::vector<size_t> word_positions;

        size_t pos = 0;
        while (pos < text.length()) {
            // Skip whitespace
            while (pos < text.length() && std::isspace(text[pos])) {
                pos++;
            }
            if (pos >= text.length()) break;

            // Extract word
            size_t word_start = pos;
            while (pos < text.length() && !std::isspace(text[pos])) {
                pos++;
            }

            words.push_back(text.substr(word_start, pos - word_start));
            word_positions.push_back(word_start);
        }

        // Build lines
        std::string current_line;
        std::vector<LinkPosition> current_links;

        for (size_t i = 0; i < words.size(); ++i) {
            const auto& word = words[i];
            size_t word_pos = word_positions[i];

            bool can_fit = current_line.empty()
                ? word.length() <= static_cast<size_t>(width)
                : current_line.length() + 1 + word.length() <= static_cast<size_t>(width);

            if (!can_fit && !current_line.empty()) {
                // Save current line
                result.push_back({current_line, current_links});
                current_line.clear();
                current_links.clear();
            }

            // Add word to current line
            if (!current_line.empty()) {
                current_line += " ";
            }
            size_t word_start_in_line = current_line.length();
            current_line += word;

            // Check if this word overlaps with any links
            for (const auto& link : modified_links) {
                size_t word_end = word_pos + word.length();

                // Check for overlap
                if (word_pos < link.end_pos && word_end > link.start_pos) {
                    // Calculate link position in current line
                    size_t link_start_in_line = word_start_in_line;
                    if (link.start_pos > word_pos) {
                        link_start_in_line += (link.start_pos - word_pos);
                    }

                    size_t link_end_in_line = word_start_in_line + word.length();
                    if (link.end_pos < word_end) {
                        link_end_in_line -= (word_end - link.end_pos);
                    }

                    // Check if link already added
                    bool already_added = false;
                    for (auto& existing : current_links) {
                        if (existing.link_index == link.link_index) {
                            // Extend existing link range
                            existing.end = link_end_in_line;
                            already_added = true;
                            break;
                        }
                    }

                    if (!already_added) {
                        LinkPosition lp;
                        lp.link_index = link.link_index;
                        lp.start = link_start_in_line;
                        lp.end = link_end_in_line;
                        current_links.push_back(lp);
                    }
                }
            }
        }

        // Add last line
        if (!current_line.empty()) {
            result.push_back({current_line, current_links});
        }

        if (result.empty()) {
            result.push_back({"", {}});
        }

        return result;
    }

    std::string add_indent(const std::string& text, int indent) {
        return std::string(indent, ' ') + text;
    }

    // Render a table with box-drawing characters
    std::vector<RenderedLine> render_table(const Table& table, int content_width, int margin) {
        std::vector<RenderedLine> lines;
        if (table.rows.empty()) return lines;

        // Calculate column widths
        size_t num_cols = 0;
        for (const auto& row : table.rows) {
            num_cols = std::max(num_cols, row.cells.size());
        }

        if (num_cols == 0) return lines;

        std::vector<int> col_widths(num_cols, 0);
        int available_width = content_width - (num_cols + 1) * 3;  // Account for borders and padding

        // First pass: calculate minimum widths
        for (const auto& row : table.rows) {
            for (size_t i = 0; i < row.cells.size() && i < num_cols; ++i) {
                int cell_len = static_cast<int>(row.cells[i].text.length());
                int max_width = available_width / static_cast<int>(num_cols);
                int cell_width = std::min(cell_len, max_width);
                col_widths[i] = std::max(col_widths[i], cell_width);
            }
        }

        // Normalize column widths
        int total_width = std::accumulate(col_widths.begin(), col_widths.end(), 0);
        if (total_width > available_width) {
            // Scale down proportionally
            for (auto& width : col_widths) {
                width = (width * available_width) / total_width;
                width = std::max(width, 5);  // Minimum column width
            }
        }

        // Helper to create separator line
        auto create_separator = [&](bool is_top, bool is_bottom, bool is_middle, bool is_header) {
            RenderedLine line;
            std::string sep = std::string(margin, ' ');

            if (is_top) {
                sep += BoxChars::TOP_LEFT;
            } else if (is_bottom) {
                sep += BoxChars::BOTTOM_LEFT;
            } else {
                sep += BoxChars::T_RIGHT;
            }

            for (size_t i = 0; i < num_cols; ++i) {
                const char* horiz = is_header ? BoxChars::HEAVY_HORIZONTAL : BoxChars::HORIZONTAL;
                sep += std::string(col_widths[i] + 2, horiz[0]);

                if (i < num_cols - 1) {
                    if (is_top) {
                        sep += BoxChars::T_DOWN;
                    } else if (is_bottom) {
                        sep += BoxChars::T_UP;
                    } else {
                        sep += BoxChars::CROSS;
                    }
                }
            }

            if (is_top) {
                sep += BoxChars::TOP_RIGHT;
            } else if (is_bottom) {
                sep += BoxChars::BOTTOM_RIGHT;
            } else {
                sep += BoxChars::T_LEFT;
            }

            line.text = sep;
            line.color_pair = COLOR_DIM;
            line.is_bold = false;
            line.is_link = false;
            line.link_index = -1;
            return line;
        };

        // Top border
        lines.push_back(create_separator(true, false, false, false));

        // Render rows
        bool first_row = true;
        for (const auto& row : table.rows) {
            bool is_header_row = first_row && table.has_header;

            // Wrap cell contents
            std::vector<std::vector<std::string>> wrapped_cells(num_cols);
            int max_cell_lines = 1;

            for (size_t i = 0; i < row.cells.size() && i < num_cols; ++i) {
                const auto& cell = row.cells[i];
                auto cell_lines = wrap_text(cell.text, col_widths[i]);
                wrapped_cells[i] = cell_lines;
                max_cell_lines = std::max(max_cell_lines, static_cast<int>(cell_lines.size()));
            }

            // Render cell lines
            for (int line_idx = 0; line_idx < max_cell_lines; ++line_idx) {
                RenderedLine line;
                std::string line_text = std::string(margin, ' ') + BoxChars::VERTICAL;

                for (size_t col_idx = 0; col_idx < num_cols; ++col_idx) {
                    std::string cell_text;
                    if (col_idx < wrapped_cells.size() && line_idx < static_cast<int>(wrapped_cells[col_idx].size())) {
                        cell_text = wrapped_cells[col_idx][line_idx];
                    }

                    // Pad to column width
                    int padding = col_widths[col_idx] - cell_text.length();
                    line_text += " " + cell_text + std::string(padding + 1, ' ') + BoxChars::VERTICAL;
                }

                line.text = line_text;
                line.color_pair = is_header_row ? COLOR_HEADING2 : COLOR_NORMAL;
                line.is_bold = is_header_row;
                line.is_link = false;
                line.link_index = -1;
                lines.push_back(line);
            }

            // Separator after header or between rows
            if (is_header_row) {
                lines.push_back(create_separator(false, false, true, true));
            }

            first_row = false;
        }

        // Bottom border
        lines.push_back(create_separator(false, true, false, false));

        return lines;
    }

    // Render an image placeholder
    std::vector<RenderedLine> render_image(const Image& img, int content_width, int margin) {
        std::vector<RenderedLine> lines;

        // Create a box for the image
        std::string img_text = "[IMG";
        if (!img.alt.empty()) {
            img_text += ": " + img.alt;
        }
        img_text += "]";

        // Truncate if too long
        if (static_cast<int>(img_text.length()) > content_width) {
            img_text = img_text.substr(0, content_width - 3) + "...]";
        }

        // Top border
        RenderedLine top;
        top.text = std::string(margin, ' ') + BoxChars::TOP_LEFT +
                   std::string(img_text.length() + 2, BoxChars::HORIZONTAL[0]) +
                   BoxChars::TOP_RIGHT;
        top.color_pair = COLOR_DIM;
        top.is_bold = false;
        top.is_link = false;
        top.link_index = -1;
        lines.push_back(top);

        // Content
        RenderedLine content;
        content.text = std::string(margin, ' ') + BoxChars::VERTICAL + " " + img_text + " " + BoxChars::VERTICAL;
        content.color_pair = COLOR_LINK;
        content.is_bold = true;
        content.is_link = false;
        content.link_index = -1;
        lines.push_back(content);

        // Dimensions if available
        if (img.width > 0 || img.height > 0) {
            std::string dims = " ";
            if (img.width > 0) dims += std::to_string(img.width) + "w";
            if (img.width > 0 && img.height > 0) dims += " × ";
            if (img.height > 0) dims += std::to_string(img.height) + "h";
            dims += " ";

            RenderedLine dim_line;
            int padding = img_text.length() + 2 - dims.length();
            dim_line.text = std::string(margin, ' ') + BoxChars::VERTICAL + dims +
                           std::string(padding, ' ') + BoxChars::VERTICAL;
            dim_line.color_pair = COLOR_DIM;
            dim_line.is_bold = false;
            dim_line.is_link = false;
            dim_line.link_index = -1;
            lines.push_back(dim_line);
        }

        // Bottom border
        RenderedLine bottom;
        bottom.text = std::string(margin, ' ') + BoxChars::BOTTOM_LEFT +
                     std::string(img_text.length() + 2, BoxChars::HORIZONTAL[0]) +
                     BoxChars::BOTTOM_RIGHT;
        bottom.color_pair = COLOR_DIM;
        bottom.is_bold = false;
        bottom.is_link = false;
        bottom.link_index = -1;
        lines.push_back(bottom);

        return lines;
    }
};

TextRenderer::TextRenderer() : pImpl(std::make_unique<Impl>()) {
    pImpl->config = RenderConfig();
}

TextRenderer::~TextRenderer() = default;

std::vector<RenderedLine> TextRenderer::render(const ParsedDocument& doc, int screen_width) {
    std::vector<RenderedLine> lines;

    int content_width = std::min(pImpl->config.max_width, screen_width - 4);
    if (content_width < 40) {
        content_width = screen_width - 4;
    }

    int margin = 0;
    if (pImpl->config.center_content && content_width < screen_width) {
        margin = (screen_width - content_width) / 2;
    }
    pImpl->config.margin_left = margin;

    if (!doc.title.empty()) {
        RenderedLine title_line;
        title_line.text = std::string(margin, ' ') + doc.title;
        title_line.color_pair = COLOR_HEADING1;
        title_line.is_bold = true;
        title_line.is_link = false;
        title_line.link_index = -1;
        lines.push_back(title_line);

        RenderedLine underline;
        underline.text = std::string(margin, ' ') + std::string(std::min((int)doc.title.length(), content_width), '=');
        underline.color_pair = COLOR_HEADING1;
        underline.is_bold = false;
        underline.is_link = false;
        underline.link_index = -1;
        lines.push_back(underline);

        RenderedLine empty;
        empty.text = "";
        empty.color_pair = COLOR_NORMAL;
        empty.is_bold = false;
        empty.is_link = false;
        empty.link_index = -1;
        lines.push_back(empty);
    }

    if (!doc.url.empty()) {
        RenderedLine url_line;
        url_line.text = std::string(margin, ' ') + "URL: " + doc.url;
        url_line.color_pair = COLOR_URL_BAR;
        url_line.is_bold = false;
        url_line.is_link = false;
        url_line.link_index = -1;
        lines.push_back(url_line);

        RenderedLine empty;
        empty.text = "";
        empty.color_pair = COLOR_NORMAL;
        empty.is_bold = false;
        empty.is_link = false;
        empty.link_index = -1;
        lines.push_back(empty);
    }

    for (const auto& elem : doc.elements) {
        int color = COLOR_NORMAL;
        bool bold = false;
        std::string prefix = "";

        switch (elem.type) {
            case ElementType::HEADING1:
                color = COLOR_HEADING1;
                bold = true;
                prefix = "# ";
                break;
            case ElementType::HEADING2:
                color = COLOR_HEADING2;
                bold = true;
                prefix = "## ";
                break;
            case ElementType::HEADING3:
                color = COLOR_HEADING3;
                bold = true;
                prefix = "### ";
                break;
            case ElementType::PARAGRAPH:
                color = COLOR_NORMAL;
                bold = false;
                break;
            case ElementType::BLOCKQUOTE:
                color = COLOR_DIM;
                prefix = "> ";
                break;
            case ElementType::LIST_ITEM:
                {
                    // Different bullets for different nesting levels
                    const char* bullets[] = {"•", "◦", "▪", "▫"};
                    int indent = elem.nesting_level * 2;
                    int bullet_idx = elem.nesting_level % 4;
                    prefix = std::string(indent, ' ') + "  " + bullets[bullet_idx] + " ";
                }
                break;
            case ElementType::ORDERED_LIST_ITEM:
                {
                    // Numbered lists with proper indentation
                    int indent = elem.nesting_level * 2;
                    prefix = std::string(indent, ' ') + "  " +
                            std::to_string(elem.list_number) + ". ";
                }
                break;
            case ElementType::TABLE:
                {
                    auto table_lines = pImpl->render_table(elem.table_data, content_width, margin);
                    lines.insert(lines.end(), table_lines.begin(), table_lines.end());

                    // Add empty line after table
                    RenderedLine empty;
                    empty.text = "";
                    empty.color_pair = COLOR_NORMAL;
                    empty.is_bold = false;
                    empty.is_link = false;
                    empty.link_index = -1;
                    lines.push_back(empty);
                    continue;
                }
            case ElementType::IMAGE:
                {
                    auto img_lines = pImpl->render_image(elem.image_data, content_width, margin);
                    lines.insert(lines.end(), img_lines.begin(), img_lines.end());

                    // Add empty line after image
                    RenderedLine empty;
                    empty.text = "";
                    empty.color_pair = COLOR_NORMAL;
                    empty.is_bold = false;
                    empty.is_link = false;
                    empty.link_index = -1;
                    lines.push_back(empty);
                    continue;
                }
            case ElementType::HORIZONTAL_RULE:
                {
                    RenderedLine hr;
                    std::string hrline(content_width, '-');
                    hr.text = std::string(margin, ' ') + hrline;
                    hr.color_pair = COLOR_DIM;
                    hr.is_bold = false;
                    hr.is_link = false;
                    hr.link_index = -1;
                    lines.push_back(hr);
                    continue;
                }
            case ElementType::HEADING4:
            case ElementType::HEADING5:
            case ElementType::HEADING6:
                color = COLOR_HEADING3;  // Use same color as H3 for H4-H6
                bold = true;
                prefix = std::string(elem.level, '#') + " ";
                break;
            default:
                break;
        }

        auto wrapped_with_links = pImpl->wrap_text_with_links(elem.text,
                                                              content_width - prefix.length(),
                                                              elem.inline_links);

        for (size_t i = 0; i < wrapped_with_links.size(); ++i) {
            const auto& [line_text, link_positions] = wrapped_with_links[i];
            RenderedLine line;

            if (i == 0) {
                line.text = std::string(margin, ' ') + prefix + line_text;
            } else {
                line.text = std::string(margin + prefix.length(), ' ') + line_text;
            }

            line.color_pair = color;
            line.is_bold = bold;

            // Store link information
            if (!link_positions.empty()) {
                line.is_link = true;
                line.link_index = link_positions[0].link_index;  // Primary link for Tab navigation

                // Adjust link positions for margin and prefix
                size_t offset = (i == 0) ? (margin + prefix.length()) : (margin + prefix.length());
                for (const auto& lp : link_positions) {
                    line.link_ranges.push_back({lp.start + offset, lp.end + offset});
                }
            } else {
                line.is_link = false;
                line.link_index = -1;
            }

            lines.push_back(line);
        }

        if (elem.type == ElementType::PARAGRAPH ||
            elem.type == ElementType::HEADING1 ||
            elem.type == ElementType::HEADING2 ||
            elem.type == ElementType::HEADING3) {
            for (int i = 0; i < pImpl->config.paragraph_spacing; ++i) {
                RenderedLine empty;
                empty.text = "";
                empty.color_pair = COLOR_NORMAL;
                empty.is_bold = false;
                empty.is_link = false;
                empty.link_index = -1;
                lines.push_back(empty);
            }
        }
    }

    // Don't show separate links section if inline links are displayed
    if (!doc.links.empty() && !pImpl->config.show_link_indicators) {
        RenderedLine separator;
        std::string sepline(content_width, '-');
        separator.text = std::string(margin, ' ') + sepline;
        separator.color_pair = COLOR_DIM;
        separator.is_bold = false;
        separator.is_link = false;
        separator.link_index = -1;
        lines.push_back(separator);

        RenderedLine links_header;
        links_header.text = std::string(margin, ' ') + "Links:";
        links_header.color_pair = COLOR_HEADING3;
        links_header.is_bold = true;
        links_header.is_link = false;
        links_header.link_index = -1;
        lines.push_back(links_header);

        RenderedLine empty;
        empty.text = "";
        empty.color_pair = COLOR_NORMAL;
        empty.is_bold = false;
        empty.is_link = false;
        empty.link_index = -1;
        lines.push_back(empty);

        for (size_t i = 0; i < doc.links.size(); ++i) {
            const auto& link = doc.links[i];
            std::string link_text = "[" + std::to_string(i) + "] " + link.text;

            auto wrapped = pImpl->wrap_text(link_text, content_width - 4);
            for (size_t j = 0; j < wrapped.size(); ++j) {
                RenderedLine link_line;
                link_line.text = std::string(margin + 2, ' ') + wrapped[j];
                link_line.color_pair = COLOR_LINK;
                link_line.is_bold = false;
                link_line.is_link = true;
                link_line.link_index = i;
                lines.push_back(link_line);
            }

            auto url_wrapped = pImpl->wrap_text(link.url, content_width - 6);
            for (const auto& url_line_text : url_wrapped) {
                RenderedLine url_line;
                url_line.text = std::string(margin + 4, ' ') + "→ " + url_line_text;
                url_line.color_pair = COLOR_DIM;
                url_line.is_bold = false;
                url_line.is_link = false;
                url_line.link_index = -1;
                lines.push_back(url_line);
            }

            lines.push_back(empty);
        }
    }

    return lines;
}

void TextRenderer::set_config(const RenderConfig& config) {
    pImpl->config = config;
}

RenderConfig TextRenderer::get_config() const {
    return pImpl->config;
}

void init_color_scheme() {
    if (has_colors()) {
        start_color();
        use_default_colors();

        init_pair(COLOR_NORMAL, COLOR_WHITE, -1);
        init_pair(COLOR_HEADING1, COLOR_CYAN, -1);
        init_pair(COLOR_HEADING2, COLOR_BLUE, -1);
        init_pair(COLOR_HEADING3, COLOR_MAGENTA, -1);
        init_pair(COLOR_LINK, COLOR_YELLOW, -1);
        init_pair(COLOR_LINK_ACTIVE, COLOR_BLACK, COLOR_YELLOW);
        init_pair(COLOR_STATUS_BAR, COLOR_BLACK, COLOR_WHITE);
        init_pair(COLOR_URL_BAR, COLOR_GREEN, -1);
        init_pair(COLOR_SEARCH_HIGHLIGHT, COLOR_BLACK, COLOR_YELLOW);
        init_pair(COLOR_DIM, COLOR_BLACK, -1);
    }
}
