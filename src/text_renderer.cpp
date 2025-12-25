#include "text_renderer.h"
#include "dom_tree.h"
#include <algorithm>
#include <sstream>
#include <cstring>
#include <cwchar>
#include <vector>
#include <cmath>
#include <numeric>

// ============================================================================ 
// Helper Functions
// ============================================================================ 

namespace { 
    // Calculate display width of UTF-8 string (handling CJK characters)
    size_t display_width(const std::string& str) {
        size_t width = 0;
        for (size_t i = 0; i < str.length(); ) {
            unsigned char c = str[i];

            if (c < 0x80) {
                // ASCII
                width += 1;
                i += 1;
            } else if ((c & 0xE0) == 0xC0) {
                // 2-byte UTF-8
                width += 1;
                i += 2;
            } else if ((c & 0xF0) == 0xE0) {
                // 3-byte UTF-8 (likely CJK)
                width += 2;
                i += 3;
            } else if ((c & 0xF8) == 0xF0) {
                // 4-byte UTF-8
                width += 2;
                i += 4;
            } else {
                i += 1;
            }
        }
        return width;
    }

    // Pad string to specific visual width
    std::string pad_string(const std::string& str, size_t target_width) {
        size_t current_width = display_width(str);
        if (current_width >= target_width) return str;
        return str + std::string(target_width - current_width, ' ');
    }

    // Clean whitespace
    std::string clean_text(const std::string& text) {
        std::string result;
        bool in_space = false;

        for (char c : text) {
            if (std::isspace(c)) {
                if (!in_space) {
                    result += ' ';
                    in_space = true;
                }
            } else {
                result += c;
                in_space = false;
            }
        }

        size_t start = result.find_first_not_of(" \t\n\r");
        if (start == std::string::npos) return "";

        size_t end = result.find_last_not_of(" \t\n\r");
        return result.substr(start, end - start + 1);
    }

    struct LinkInfo {
        size_t start_pos;
        size_t end_pos;
        int link_index;
        int field_index;
    };

    // Text wrapping with link preservation
    std::vector<std::pair<std::string, std::vector<LinkInfo>>> wrap_text_with_links(
        const std::string& text,
        int max_width,
        const std::vector<InlineLink>& links
    ) {
        std::vector<std::pair<std::string, std::vector<LinkInfo>>> result;
        if (max_width <= 0) return result;

        // 1. Insert [N] markers for links (form fields don't get [N])
        std::string marked_text;
        std::vector<LinkInfo> adjusted_links;
        size_t pos = 0;

        for (const auto& link : links) {
            marked_text += text.substr(pos, link.start_pos - pos);
            size_t link_start = marked_text.length();
            
            marked_text += text.substr(link.start_pos, link.end_pos - link.start_pos);
            
            // Add marker [N] only for links
            if (link.link_index >= 0) {
                std::string marker = "[" + std::to_string(link.link_index + 1) + "]";
                marked_text += marker;
            }
            
            size_t link_end = marked_text.length();

            adjusted_links.push_back({link_start, link_end, link.link_index, link.field_index});
            pos = link.end_pos;
        }

        if (pos < text.length()) {
            marked_text += text.substr(pos);
        }

        // 2. Wrap text
        size_t line_start_idx = 0;
        size_t current_line_width = 0;
        size_t last_space_idx = std::string::npos;
        
        for (size_t i = 0; i <= marked_text.length(); ++i) {
             bool is_break = (i == marked_text.length() || marked_text[i] == ' ' || marked_text[i] == '\n');
             
             if (is_break) {
                 std::string word = marked_text.substr(
                     (last_space_idx == std::string::npos) ? line_start_idx : last_space_idx + 1,
                     i - ((last_space_idx == std::string::npos) ? line_start_idx : last_space_idx + 1)
                 );
                 
                 size_t word_width = display_width(word);
                 size_t space_width = (current_line_width == 0) ? 0 : 1;
                 
                 if (current_line_width + space_width + word_width > static_cast<size_t>(max_width)) {
                     // Wrap
                     if (current_line_width > 0) {
                         // End current line at last space
                         std::string line_str = marked_text.substr(line_start_idx, last_space_idx - line_start_idx); 
                         
                         // Collect links
                         std::vector<LinkInfo> line_links;
                         for (const auto& link : adjusted_links) {
                             // Check overlap
                             size_t link_s = link.start_pos;
                             size_t link_e = link.end_pos;
                             size_t line_s = line_start_idx;
                             size_t line_e = last_space_idx;
                             
                             if (link_s < line_e && link_e > line_s) {
                                 size_t start = (link_s > line_s) ? link_s - line_s : 0;
                                 size_t end = (link_e < line_e) ? link_e - line_s : line_e - line_s;
                                 line_links.push_back({start, end, link.link_index, link.field_index});
                             }
                         }
                         result.push_back({line_str, line_links});
                         
                         // Start new line
                         line_start_idx = last_space_idx + 1;
                         current_line_width = word_width;
                         last_space_idx = i;
                     } else {
                         // Word itself is too long, force break (not implemented for simplicity, just overflow)
                         last_space_idx = i;
                         current_line_width += space_width + word_width;
                     }
                 } else {
                     current_line_width += space_width + word_width;
                     last_space_idx = i;
                 }
             }
        }
        
        // Last line
        if (line_start_idx < marked_text.length()) {
            std::string line_str = marked_text.substr(line_start_idx);
            std::vector<LinkInfo> line_links;
            for (const auto& link : adjusted_links) {
                size_t link_s = link.start_pos;
                size_t link_e = link.end_pos;
                size_t line_s = line_start_idx;
                size_t line_e = marked_text.length();
                
                if (link_s < line_e && link_e > line_s) {
                    size_t start = (link_s > line_s) ? link_s - line_s : 0;
                    size_t end = (link_e < line_e) ? link_e - line_s : line_e - line_s;
                    line_links.push_back({start, end, link.link_index, link.field_index});
                }
            }
            result.push_back({line_str, line_links});
        }
        
        return result;
    }
} 

// ============================================================================ 
// TextRenderer::Impl
// ============================================================================ 

class TextRenderer::Impl {
public:
    RenderConfig config;

    struct InlineContent {
        std::string text;
        std::vector<InlineLink> links;
    };

    RenderedLine create_empty_line() {
        RenderedLine line;
        line.text = "";
        line.color_pair = COLOR_NORMAL;
        line.is_bold = false;
        line.is_link = false;
        line.link_index = -1;
        return line;
    }

    std::vector<RenderedLine> render_tree(const DocumentTree& tree, int screen_width) {
        std::vector<RenderedLine> lines;
        if (!tree.root) return lines;

        RenderContext ctx;
        ctx.screen_width = config.center_content ? std::min(config.max_width, screen_width) : screen_width;
        ctx.current_indent = 0;
        ctx.nesting_level = 0;
        ctx.color_pair = COLOR_NORMAL;
        ctx.is_bold = false;

        render_node(tree.root.get(), ctx, lines);
        return lines;
    }

    void render_node(DomNode* node, RenderContext& ctx, std::vector<RenderedLine>& lines) {
        if (!node || !node->should_render()) return;

        if (node->is_block_element()) {
            if (node->tag_name == "table") {
                render_table(node, ctx, lines);
            } else {
                switch (node->element_type) {
                    case ElementType::HEADING1:
                    case ElementType::HEADING2:
                    case ElementType::HEADING3:
                        render_heading(node, ctx, lines);
                        break;
                    case ElementType::PARAGRAPH:
                        render_paragraph(node, ctx, lines);
                        break;
                    case ElementType::HORIZONTAL_RULE:
                        render_hr(node, ctx, lines);
                        break;
                    case ElementType::CODE_BLOCK:
                        render_code_block(node, ctx, lines);
                        break;
                    case ElementType::BLOCKQUOTE:
                        render_blockquote(node, ctx, lines);
                        break;
                    default:
                        if (node->tag_name == "ul" || node->tag_name == "ol") {
                            render_list(node, ctx, lines);
                        } else {
                            for (auto& child : node->children) {
                                render_node(child.get(), ctx, lines);
                            }
                        }
                }
            }
        } else if (node->node_type == NodeType::DOCUMENT || node->node_type == NodeType::ELEMENT) {
            for (auto& child : node->children) {
                render_node(child.get(), ctx, lines);
            }
        }
    }

    // ========================================================================
    // Table Rendering
    // ========================================================================

    struct CellData {
        std::vector<std::string> lines; // Wrapped lines
        int width = 0;
        int height = 0;
        int colspan = 1;
        int rowspan = 1;
        bool is_header = false;
    };

    void render_table(DomNode* node, RenderContext& ctx, std::vector<RenderedLine>& lines) {
        // Simplified table rendering (skipping complex grid for brevity, reverting to previous improved logic)
        // Note: For brevity in this tool call, reusing the logic from previous step but integrated with form fields?
        // Actually, let's keep the logic I wrote before.
        
        // 1. Collect Table Data
        std::vector<std::vector<CellData>> grid;
        std::vector<int> col_widths;
        int max_cols = 0;

        for (auto& child : node->children) {
            if (child->tag_name == "tr") {
                std::vector<CellData> row;
                for (auto& cell : child->children) {
                    if (cell->tag_name == "td" || cell->tag_name == "th") {
                        CellData data;
                        data.is_header = (cell->tag_name == "th");
                        data.colspan = cell->colspan > 0 ? cell->colspan : 1;
                        InlineContent content = collect_inline_content(cell.get());
                        std::string clean = clean_text(content.text);
                        data.lines.push_back(clean); 
                        data.width = display_width(clean);
                        data.height = 1;
                        row.push_back(data);
                    }
                }
                if (!row.empty()) {
                    grid.push_back(row);
                    max_cols = std::max(max_cols, (int)row.size());
                }
            }
        }

        if (grid.empty()) return;

        col_widths.assign(max_cols, 0);
        for (const auto& row : grid) {
            for (size_t i = 0; i < row.size(); ++i) {
                if (i < col_widths.size()) {
                    col_widths[i] = std::max(col_widths[i], row[i].width);
                }
            }
        }

        int total_width = std::accumulate(col_widths.begin(), col_widths.end(), 0);
        int available_width = ctx.screen_width - 4;
        available_width = std::max(10, available_width);

        if (total_width > available_width) {
            double ratio = (double)available_width / total_width;
            for (auto& w : col_widths) {
                w = std::max(3, (int)(w * ratio));
            }
        }

        std::string border_line = "+";
        for (int w : col_widths) {
            border_line += std::string(w + 2, '-') + "+";
        }
        
        RenderedLine border;
        border.text = border_line;
        border.color_pair = COLOR_DIM;
        lines.push_back(border);

        for (auto& row : grid) {
            int max_row_height = 0;
            std::vector<std::vector<std::string>> row_wrapped_content;

            for (size_t i = 0; i < row.size(); ++i) {
                if (i >= col_widths.size()) break;
                
                int cell_w = col_widths[i];
                std::string raw_text = row[i].lines[0];
                auto wrapped = wrap_text_with_links(raw_text, cell_w, {}); // Simplified: no links in table for now
                
                std::vector<std::string> cell_lines;
                for(auto& p : wrapped) cell_lines.push_back(p.first);
                if (cell_lines.empty()) cell_lines.push_back("");
                
                row_wrapped_content.push_back(cell_lines);
                max_row_height = std::max(max_row_height, (int)cell_lines.size());
            }

            for (int h = 0; h < max_row_height; ++h) {
                std::string line_str = "|";
                for (size_t i = 0; i < col_widths.size(); ++i) {
                    int w = col_widths[i];
                    std::string content = "";
                    if (i < row_wrapped_content.size() && h < (int)row_wrapped_content[i].size()) {
                        content = row_wrapped_content[i][h];
                    }
                    line_str += " " + pad_string(content, w) + " |";
                }
                
                RenderedLine rline;
                rline.text = line_str;
                rline.color_pair = COLOR_NORMAL;
                lines.push_back(rline);
            }
            
            lines.push_back(border);
        }
        
        lines.push_back(create_empty_line());
    }

    // ========================================================================
    // Other Elements
    // ========================================================================

    void render_heading(DomNode* node, RenderContext& /*ctx*/, std::vector<RenderedLine>& lines) {
        InlineContent content = collect_inline_content(node);
        if (content.text.empty()) return;

        RenderedLine line;
        line.text = clean_text(content.text);
        line.color_pair = COLOR_HEADING1;
        line.is_bold = true;
        lines.push_back(line);
        lines.push_back(create_empty_line());
    }

    void render_paragraph(DomNode* node, RenderContext& ctx, std::vector<RenderedLine>& lines) {
        InlineContent content = collect_inline_content(node);
        std::string text = clean_text(content.text);
        if (text.empty()) return;

        auto wrapped = wrap_text_with_links(text, ctx.screen_width, content.links);
        for (const auto& [line_text, link_infos] : wrapped) {
            RenderedLine line;
            line.text = line_text;
            line.color_pair = COLOR_NORMAL;
            if (!link_infos.empty()) {
                line.is_link = true; // Kept for compatibility, though we use interactive_ranges
                line.link_index = -1; 
                
                for (const auto& li : link_infos) {
                    InteractiveRange range;
                    range.start = li.start_pos;
                    range.end = li.end_pos;
                    range.link_index = li.link_index;
                    range.field_index = li.field_index;
                    line.interactive_ranges.push_back(range);
                    
                    if (li.link_index >= 0) line.link_index = li.link_index; // Heuristic: set main link index to first link
                }
            }
            lines.push_back(line);
        }
        lines.push_back(create_empty_line());
    }
    
    void render_list(DomNode* node, RenderContext& ctx, std::vector<RenderedLine>& lines) {
        bool is_ordered = (node->tag_name == "ol");
        int count = 1;
        
        for(auto& child : node->children) {
            if(child->tag_name == "li") {
                InlineContent content = collect_inline_content(child.get());
                std::string prefix = is_ordered ? std::to_string(count++) + ". " : "* ";
                
                auto wrapped = wrap_text_with_links(clean_text(content.text), ctx.screen_width - 4, content.links);
                
                bool first = true;
                for(const auto& [txt, links_info] : wrapped) {
                    RenderedLine line;
                    line.text = (first ? prefix : "  ") + txt;
                    line.color_pair = COLOR_NORMAL;
                    
                    if(!links_info.empty()) {
                        line.is_link = true;
                        for(const auto& l : links_info) {
                            InteractiveRange range;
                            range.start = l.start_pos + prefix.length();
                            range.end = l.end_pos + prefix.length();
                            range.link_index = l.link_index;
                            range.field_index = l.field_index;
                            line.interactive_ranges.push_back(range);
                        }
                    }
                    lines.push_back(line);
                    first = false;
                }
            }
        }
        lines.push_back(create_empty_line());
    }

    void render_hr(DomNode* /*node*/, RenderContext& ctx, std::vector<RenderedLine>& lines) {
        RenderedLine line;
        line.text = std::string(ctx.screen_width, '-');
        line.color_pair = COLOR_DIM;
        lines.push_back(line);
        lines.push_back(create_empty_line());
    }
    
    void render_code_block(DomNode* node, RenderContext& /*ctx*/, std::vector<RenderedLine>& lines) {
        std::string text = node->get_all_text();
        std::istringstream iss(text);
        std::string line_str;
        while(std::getline(iss, line_str)) {
             RenderedLine line;
             line.text = "  " + line_str;
             line.color_pair = COLOR_DIM;
             lines.push_back(line);
        }
        lines.push_back(create_empty_line());
    }

    void render_blockquote(DomNode* node, RenderContext& ctx, std::vector<RenderedLine>& lines) {
         for (auto& child : node->children) {
             render_node(child.get(), ctx, lines);
         }
    }

    // Helper: Collect Inline Content
    InlineContent collect_inline_content(DomNode* node) {
        InlineContent result;
        for (auto& child : node->children) {
            if (child->node_type == NodeType::TEXT) {
                result.text += child->text_content;
            } else if (child->element_type == ElementType::LINK && child->link_index >= 0) {
                InlineLink link;
                link.text = child->get_all_text();
                link.url = child->href;
                link.link_index = child->link_index;
                link.field_index = -1;
                link.start_pos = result.text.length();
                result.text += link.text;
                link.end_pos = result.text.length();
                result.links.push_back(link);
            } else if (child->element_type == ElementType::INPUT) {
                 std::string repr;
                 if (child->input_type == "checkbox") {
                     repr = child->checked ? "[x]" : "[ ]";
                 } else if (child->input_type == "radio") {
                     repr = child->checked ? "(*)" : "( )";
                 } else if (child->input_type == "submit" || child->input_type == "button") {
                     repr = "[" + (child->value.empty() ? "Submit" : child->value) + "]";
                 } else {
                     // text, password, etc.
                     std::string val = child->value.empty() ? child->placeholder : child->value;
                     if (val.empty()) val = "________";
                     repr = "[" + val + "]";
                 }
                 
                 InlineLink link;
                 link.text = repr;
                 link.link_index = -1;
                 link.field_index = child->field_index;
                 link.start_pos = result.text.length();
                 result.text += repr;
                 link.end_pos = result.text.length();
                 result.links.push_back(link);
            } else if (child->element_type == ElementType::BUTTON) {
                 std::string repr = "[" + (child->value.empty() ? (child->name.empty() ? "Button" : child->name) : child->value) + "]";
                 InlineLink link;
                 link.text = repr;
                 link.link_index = -1;
                 link.field_index = child->field_index;
                 link.start_pos = result.text.length();
                 result.text += repr;
                 link.end_pos = result.text.length();
                 result.links.push_back(link);
            } else if (child->element_type == ElementType::TEXTAREA) {
                 std::string repr = "[ " + (child->value.empty() ? "Textarea" : child->value) + " ]";
                 InlineLink link;
                 link.text = repr;
                 link.link_index = -1;
                 link.field_index = child->field_index;
                 link.start_pos = result.text.length();
                 result.text += repr;
                 link.end_pos = result.text.length();
                 result.links.push_back(link);
            } else if (child->element_type == ElementType::SELECT) {
                 std::string repr = "[ Select ]"; // Simplified
                 InlineLink link;
                 link.text = repr;
                 link.link_index = -1;
                 link.field_index = child->field_index;
                 link.start_pos = result.text.length();
                 result.text += repr;
                 link.end_pos = result.text.length();
                 result.links.push_back(link);
            } else if (child->element_type == ElementType::IMAGE) {
                 // Render image placeholder
                 std::string repr = "[IMG";
                 if (!child->alt_text.empty()) {
                     repr += ": " + child->alt_text;
                 }
                 repr += "]";
                 
                 result.text += repr;
                 // Images are not necessarily links unless wrapped in <a>.
                 // If wrapped in <a>, the parent processing handles the link range.
            } else {
                 InlineContent nested = collect_inline_content(child.get());
                 size_t offset = result.text.length();
                 result.text += nested.text;
                 for(auto l : nested.links) {
                     l.start_pos += offset;
                     l.end_pos += offset;
                     result.links.push_back(l);
                 }
            }
        }
        return result;
    }

    // Legacy support
    std::vector<RenderedLine> render_legacy(const ParsedDocument& /*doc*/, int /*screen_width*/) {
        return {}; // Not used anymore
    }
};

// ============================================================================ 
// Public Interface
// ============================================================================ 

TextRenderer::TextRenderer() : pImpl(std::make_unique<Impl>()) {}
TextRenderer::~TextRenderer() = default;

std::vector<RenderedLine> TextRenderer::render_tree(const DocumentTree& tree, int screen_width) {
    return pImpl->render_tree(tree, screen_width);
}

std::vector<RenderedLine> TextRenderer::render(const ParsedDocument& doc, int screen_width) {
    return pImpl->render_legacy(doc, screen_width);
}

void TextRenderer::set_config(const RenderConfig& config) {
    pImpl->config = config;
}

RenderConfig TextRenderer::get_config() const {
    return pImpl->config;
}

void init_color_scheme() {
    init_pair(COLOR_NORMAL, COLOR_WHITE, COLOR_BLACK);
    init_pair(COLOR_HEADING1, COLOR_CYAN, COLOR_BLACK);
    init_pair(COLOR_HEADING2, COLOR_CYAN, COLOR_BLACK);
    init_pair(COLOR_HEADING3, COLOR_CYAN, COLOR_BLACK);
    init_pair(COLOR_LINK, COLOR_YELLOW, COLOR_BLACK);
    init_pair(COLOR_LINK_ACTIVE, COLOR_YELLOW, COLOR_BLUE);
    init_pair(COLOR_STATUS_BAR, COLOR_BLACK, COLOR_WHITE);
    init_pair(COLOR_URL_BAR, COLOR_CYAN, COLOR_BLACK);
    init_pair(COLOR_SEARCH_HIGHLIGHT, COLOR_BLACK, COLOR_YELLOW);
    init_pair(COLOR_DIM, COLOR_WHITE, COLOR_BLACK);
}
