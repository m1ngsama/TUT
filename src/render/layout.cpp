#include "layout.h"
#include "decorations.h"
#include "image.h"
#include <sstream>
#include <algorithm>

namespace tut {

// ==================== LayoutEngine ====================

LayoutEngine::LayoutEngine(int viewport_width)
    : viewport_width_(viewport_width)
    , content_width_(viewport_width - MARGIN_LEFT - MARGIN_RIGHT)
{
}

LayoutResult LayoutEngine::layout(const DocumentTree& doc) {
    LayoutResult result;
    result.title = doc.title;
    result.url = doc.url;

    if (!doc.root) {
        return result;
    }

    Context ctx;
    layout_node(doc.root.get(), ctx, result.blocks);

    // 计算总行数并收集链接和字段位置
    int total = 0;

    // 预分配位置数组
    size_t num_links = doc.links.size();
    size_t num_fields = doc.form_fields.size();
    result.link_positions.resize(num_links, {-1, -1});
    result.field_lines.resize(num_fields, -1);

    for (const auto& block : result.blocks) {
        total += block.margin_top;

        for (const auto& line : block.lines) {
            for (const auto& span : line.spans) {
                // 记录链接位置
                if (span.link_index >= 0 && span.link_index < static_cast<int>(num_links)) {
                    auto& pos = result.link_positions[span.link_index];
                    if (pos.start_line < 0) {
                        pos.start_line = total;
                    }
                    pos.end_line = total;
                }
                // 记录字段位置
                if (span.field_index >= 0 && span.field_index < static_cast<int>(num_fields)) {
                    if (result.field_lines[span.field_index] < 0) {
                        result.field_lines[span.field_index] = total;
                    }
                }
            }
            total++;
        }

        total += block.margin_bottom;
    }
    result.total_lines = total;

    return result;
}

void LayoutEngine::layout_node(const DomNode* node, Context& ctx, std::vector<LayoutBlock>& blocks) {
    if (!node || !node->should_render()) {
        return;
    }

    if (node->node_type == NodeType::DOCUMENT) {
        for (const auto& child : node->children) {
            layout_node(child.get(), ctx, blocks);
        }
        return;
    }

    // 处理容器元素（html, body, div, form等）- 递归处理子节点
    if (node->tag_name == "html" || node->tag_name == "body" ||
        node->tag_name == "head" || node->tag_name == "main" ||
        node->tag_name == "article" || node->tag_name == "section" ||
        node->tag_name == "div" || node->tag_name == "header" ||
        node->tag_name == "footer" || node->tag_name == "nav" ||
        node->tag_name == "aside" || node->tag_name == "form" ||
        node->tag_name == "fieldset") {
        for (const auto& child : node->children) {
            layout_node(child.get(), ctx, blocks);
        }
        return;
    }

    // 处理表单内联元素
    if (node->element_type == ElementType::INPUT ||
        node->element_type == ElementType::BUTTON ||
        node->element_type == ElementType::TEXTAREA ||
        node->element_type == ElementType::SELECT) {
        layout_form_element(node, ctx, blocks);
        return;
    }

    // 处理图片元素
    if (node->element_type == ElementType::IMAGE) {
        layout_image_element(node, ctx, blocks);
        return;
    }

    if (node->is_block_element()) {
        layout_block_element(node, ctx, blocks);
    }
    // 内联元素在块级元素内部处理
}

void LayoutEngine::layout_block_element(const DomNode* node, Context& ctx, std::vector<LayoutBlock>& blocks) {
    LayoutBlock block;
    block.type = node->element_type;

    // 设置边距
    switch (node->element_type) {
        case ElementType::HEADING1:
            block.margin_top = 1;
            block.margin_bottom = 1;
            break;
        case ElementType::HEADING2:
        case ElementType::HEADING3:
            block.margin_top = 1;
            block.margin_bottom = 0;
            break;
        case ElementType::PARAGRAPH:
            block.margin_top = 0;
            block.margin_bottom = 1;
            break;
        case ElementType::LIST_ITEM:
        case ElementType::ORDERED_LIST_ITEM:
            block.margin_top = 0;
            block.margin_bottom = 0;
            break;
        case ElementType::BLOCKQUOTE:
            block.margin_top = 1;
            block.margin_bottom = 1;
            break;
        case ElementType::CODE_BLOCK:
            block.margin_top = 1;
            block.margin_bottom = 1;
            break;
        case ElementType::HORIZONTAL_RULE:
            block.margin_top = 1;
            block.margin_bottom = 1;
            break;
        default:
            block.margin_top = 0;
            block.margin_bottom = 0;
            break;
    }

    // 处理特殊块元素
    if (node->element_type == ElementType::HORIZONTAL_RULE) {
        // 水平线
        LayoutLine line;
        StyledSpan hr_span;
        hr_span.text = make_horizontal_line(content_width_, chars::SGL_HORIZONTAL);
        hr_span.fg = colors::DIVIDER;
        line.spans.push_back(hr_span);
        line.indent = MARGIN_LEFT;
        block.lines.push_back(line);
        blocks.push_back(block);
        return;
    }

    // 检查是否是列表容器（通过tag_name判断）
    if (node->tag_name == "ul" || node->tag_name == "ol") {
        // 列表：递归处理子元素
        ctx.list_depth++;
        bool is_ordered = (node->tag_name == "ol");
        if (is_ordered) {
            ctx.ordered_list_counter = 1;
        }

        for (const auto& child : node->children) {
            if (child->element_type == ElementType::LIST_ITEM ||
                child->element_type == ElementType::ORDERED_LIST_ITEM) {
                layout_block_element(child.get(), ctx, blocks);
                if (is_ordered) {
                    ctx.ordered_list_counter++;
                }
            }
        }

        ctx.list_depth--;
        return;
    }

    if (node->element_type == ElementType::BLOCKQUOTE) {
        ctx.in_blockquote = true;
    }

    if (node->element_type == ElementType::CODE_BLOCK) {
        ctx.in_pre = true;
    }

    // 收集内联内容
    std::vector<StyledSpan> spans;

    // 列表项的标记
    if (node->element_type == ElementType::LIST_ITEM) {
        StyledSpan marker;
        marker.text = get_list_marker(ctx.list_depth, ctx.ordered_list_counter > 0, ctx.ordered_list_counter);
        marker.fg = colors::FG_SECONDARY;
        spans.push_back(marker);
    }

    collect_inline_content(node, ctx, spans);

    if (node->element_type == ElementType::BLOCKQUOTE) {
        ctx.in_blockquote = false;
    }

    if (node->element_type == ElementType::CODE_BLOCK) {
        ctx.in_pre = false;
    }

    // 计算缩进
    int indent = MARGIN_LEFT;
    if (ctx.list_depth > 0) {
        indent += ctx.list_depth * 2;
    }
    if (ctx.in_blockquote) {
        indent += 2;
    }

    // 换行
    int available_width = content_width_ - (indent - MARGIN_LEFT);
    if (ctx.in_pre) {
        // 预格式化文本不换行
        for (const auto& span : spans) {
            LayoutLine line;
            line.indent = indent;
            line.spans.push_back(span);
            block.lines.push_back(line);
        }
    } else {
        block.lines = wrap_text(spans, available_width, indent);
    }

    // 引用块添加边框
    if (node->element_type == ElementType::BLOCKQUOTE && !block.lines.empty()) {
        for (auto& line : block.lines) {
            StyledSpan border;
            border.text = chars::QUOTE_LEFT + std::string(" ");
            border.fg = colors::QUOTE_BORDER;
            line.spans.insert(line.spans.begin(), border);
        }
    }

    if (!block.lines.empty()) {
        blocks.push_back(block);
    }

    // 处理子块元素
    for (const auto& child : node->children) {
        if (child->is_block_element()) {
            layout_node(child.get(), ctx, blocks);
        }
    }
}

void LayoutEngine::layout_form_element(const DomNode* node, Context& /*ctx*/, std::vector<LayoutBlock>& blocks) {
    LayoutBlock block;
    block.type = node->element_type;
    block.margin_top = 0;
    block.margin_bottom = 0;

    LayoutLine line;
    line.indent = MARGIN_LEFT;

    if (node->element_type == ElementType::INPUT) {
        // 渲染输入框
        std::string input_type = node->input_type;

        if (input_type == "submit" || input_type == "button") {
            // 按钮样式: [ Submit ]
            std::string label = node->value.empty() ? "Submit" : node->value;
            StyledSpan span;
            span.text = "[ " + label + " ]";
            span.fg = colors::INPUT_FOCUS;
            span.bg = colors::INPUT_BG;
            span.attrs = ATTR_BOLD;
            span.field_index = node->field_index;
            line.spans.push_back(span);
        } else if (input_type == "checkbox") {
            // 复选框: [x] 或 [ ]
            StyledSpan span;
            span.text = node->checked ? "[x]" : "[ ]";
            span.fg = colors::INPUT_FOCUS;
            span.field_index = node->field_index;
            line.spans.push_back(span);

            // 添加标签（如果有name）
            if (!node->name.empty()) {
                StyledSpan label;
                label.text = " " + node->name;
                label.fg = colors::FG_PRIMARY;
                line.spans.push_back(label);
            }
        } else if (input_type == "radio") {
            // 单选框: (o) 或 ( )
            StyledSpan span;
            span.text = node->checked ? "(o)" : "( )";
            span.fg = colors::INPUT_FOCUS;
            span.field_index = node->field_index;
            line.spans.push_back(span);

            if (!node->name.empty()) {
                StyledSpan label;
                label.text = " " + node->name;
                label.fg = colors::FG_PRIMARY;
                line.spans.push_back(label);
            }
        } else {
            // 文本输入框: [placeholder____]
            std::string display_text;
            if (!node->value.empty()) {
                display_text = node->value;
            } else if (!node->placeholder.empty()) {
                display_text = node->placeholder;
            } else {
                display_text = "";
            }

            // 限制显示宽度
            int field_width = 20;
            if (display_text.length() > static_cast<size_t>(field_width)) {
                display_text = display_text.substr(0, field_width - 1) + "…";
            } else {
                display_text += std::string(field_width - display_text.length(), '_');
            }

            StyledSpan span;
            span.text = "[" + display_text + "]";
            span.fg = node->value.empty() ? colors::FG_DIM : colors::FG_PRIMARY;
            span.bg = colors::INPUT_BG;
            span.field_index = node->field_index;
            line.spans.push_back(span);
        }
    } else if (node->element_type == ElementType::BUTTON) {
        // 按钮
        std::string label = node->get_all_text();
        if (label.empty()) {
            label = node->value.empty() ? "Button" : node->value;
        }
        StyledSpan span;
        span.text = "[ " + label + " ]";
        span.fg = colors::INPUT_FOCUS;
        span.bg = colors::INPUT_BG;
        span.attrs = ATTR_BOLD;
        span.field_index = node->field_index;
        line.spans.push_back(span);
    } else if (node->element_type == ElementType::TEXTAREA) {
        // 文本区域
        std::string content = node->value.empty() ? node->placeholder : node->value;
        if (content.empty()) {
            content = "(empty)";
        }

        StyledSpan span;
        span.text = "[" + content + "]";
        span.fg = colors::FG_PRIMARY;
        span.bg = colors::INPUT_BG;
        span.field_index = node->field_index;
        line.spans.push_back(span);
    } else if (node->element_type == ElementType::SELECT) {
        // 下拉选择
        StyledSpan span;
        span.text = "[▼ Select]";
        span.fg = colors::INPUT_FOCUS;
        span.bg = colors::INPUT_BG;
        span.field_index = node->field_index;
        line.spans.push_back(span);
    }

    if (!line.spans.empty()) {
        block.lines.push_back(line);
        blocks.push_back(block);
    }
}

void LayoutEngine::layout_image_element(const DomNode* node, Context& /*ctx*/, std::vector<LayoutBlock>& blocks) {
    LayoutBlock block;
    block.type = ElementType::IMAGE;
    block.margin_top = 0;
    block.margin_bottom = 1;

    LayoutLine line;
    line.indent = MARGIN_LEFT;

    // 生成图片占位符
    std::string placeholder = make_image_placeholder(node->alt_text, node->img_src);

    StyledSpan span;
    span.text = placeholder;
    span.fg = colors::FG_DIM;  // 使用较暗的颜色表示占位符
    span.attrs = ATTR_NONE;

    line.spans.push_back(span);
    block.lines.push_back(line);
    blocks.push_back(block);
}

void LayoutEngine::collect_inline_content(const DomNode* node, Context& ctx, std::vector<StyledSpan>& spans) {
    if (!node) return;

    if (node->node_type == NodeType::TEXT) {
        layout_text(node, ctx, spans);
        return;
    }

    if (node->is_inline_element() || node->node_type == NodeType::ELEMENT) {
        // 设置样式
        uint32_t fg = get_element_fg_color(node->element_type);
        uint8_t attrs = get_element_attrs(node->element_type);

        // 处理链接
        int link_idx = node->link_index;

        // 递归处理子节点
        for (const auto& child : node->children) {
            if (child->node_type == NodeType::TEXT) {
                StyledSpan span;
                span.text = child->text_content;
                span.fg = fg;
                span.attrs = attrs;
                span.link_index = link_idx;

                if (ctx.in_blockquote) {
                    span.fg = colors::QUOTE_FG;
                }

                spans.push_back(span);
            } else if (!child->is_block_element()) {
                collect_inline_content(child.get(), ctx, spans);
            }
        }
    }
}

void LayoutEngine::layout_text(const DomNode* node, Context& ctx, std::vector<StyledSpan>& spans) {
    if (!node || node->text_content.empty()) return;

    StyledSpan span;
    span.text = node->text_content;
    span.fg = colors::FG_PRIMARY;

    if (ctx.in_blockquote) {
        span.fg = colors::QUOTE_FG;
    }

    spans.push_back(span);
}

std::vector<LayoutLine> LayoutEngine::wrap_text(const std::vector<StyledSpan>& spans, int available_width, int indent) {
    std::vector<LayoutLine> lines;

    if (spans.empty()) {
        return lines;
    }

    LayoutLine current_line;
    current_line.indent = indent;
    size_t current_width = 0;

    for (const auto& span : spans) {
        // 分词处理
        std::istringstream iss(span.text);
        std::string word;
        bool first_word = true;

        while (iss >> word) {
            size_t word_width = Unicode::display_width(word);

            // 检查是否需要换行
            if (current_width > 0 && current_width + 1 + word_width > static_cast<size_t>(available_width)) {
                // 当前行已满，开始新行
                if (!current_line.spans.empty()) {
                    lines.push_back(current_line);
                }
                current_line = LayoutLine();
                current_line.indent = indent;
                current_width = 0;
                first_word = true;
            }

            // 添加空格（如果不是行首）
            if (current_width > 0 && !first_word) {
                if (!current_line.spans.empty()) {
                    current_line.spans.back().text += " ";
                    current_width += 1;
                }
            }

            // 添加单词
            StyledSpan word_span = span;
            word_span.text = word;
            current_line.spans.push_back(word_span);
            current_width += word_width;
            first_word = false;
        }
    }

    // 添加最后一行
    if (!current_line.spans.empty()) {
        lines.push_back(current_line);
    }

    return lines;
}

uint32_t LayoutEngine::get_element_fg_color(ElementType type) const {
    switch (type) {
        case ElementType::HEADING1:
            return colors::H1_FG;
        case ElementType::HEADING2:
            return colors::H2_FG;
        case ElementType::HEADING3:
        case ElementType::HEADING4:
        case ElementType::HEADING5:
        case ElementType::HEADING6:
            return colors::H3_FG;
        case ElementType::LINK:
            return colors::LINK_FG;
        case ElementType::CODE_BLOCK:
            return colors::CODE_FG;
        case ElementType::BLOCKQUOTE:
            return colors::QUOTE_FG;
        default:
            return colors::FG_PRIMARY;
    }
}

uint8_t LayoutEngine::get_element_attrs(ElementType type) const {
    switch (type) {
        case ElementType::HEADING1:
        case ElementType::HEADING2:
        case ElementType::HEADING3:
        case ElementType::HEADING4:
        case ElementType::HEADING5:
        case ElementType::HEADING6:
            return ATTR_BOLD;
        case ElementType::LINK:
            return ATTR_UNDERLINE;
        default:
            return ATTR_NONE;
    }
}

std::string LayoutEngine::get_list_marker(int depth, bool ordered, int counter) const {
    if (ordered) {
        return std::to_string(counter) + ". ";
    }

    // 不同层级使用不同的标记
    switch ((depth - 1) % 3) {
        case 0: return std::string(chars::BULLET) + " ";
        case 1: return std::string(chars::BULLET_HOLLOW) + " ";
        case 2: return std::string(chars::BULLET_SQUARE) + " ";
        default: return std::string(chars::BULLET) + " ";
    }
}

// ==================== DocumentRenderer ====================

DocumentRenderer::DocumentRenderer(FrameBuffer& buffer)
    : buffer_(buffer)
{
}

void DocumentRenderer::render(const LayoutResult& layout, int scroll_offset, const RenderContext& ctx) {
    int buffer_height = buffer_.height();
    int y = 0;  // 缓冲区行位置
    int doc_line = 0;  // 文档行位置

    for (const auto& block : layout.blocks) {
        // 处理上边距
        for (int i = 0; i < block.margin_top; ++i) {
            if (doc_line >= scroll_offset && y < buffer_height) {
                // 空行
                y++;
            }
            doc_line++;
        }

        // 渲染内容行
        for (const auto& line : block.lines) {
            if (doc_line >= scroll_offset) {
                if (y >= buffer_height) {
                    return;  // 超出视口
                }
                render_line(line, y, doc_line, ctx);
                y++;
            }
            doc_line++;
        }

        // 处理下边距
        for (int i = 0; i < block.margin_bottom; ++i) {
            if (doc_line >= scroll_offset && y < buffer_height) {
                // 空行
                y++;
            }
            doc_line++;
        }
    }
}

int DocumentRenderer::find_match_at(const SearchContext* search, int doc_line, int col) const {
    if (!search || !search->enabled || search->matches.empty()) {
        return -1;
    }

    for (size_t i = 0; i < search->matches.size(); ++i) {
        const auto& m = search->matches[i];
        if (m.line == doc_line && col >= m.start_col && col < m.start_col + m.length) {
            return static_cast<int>(i);
        }
    }
    return -1;
}

void DocumentRenderer::render_line(const LayoutLine& line, int y, int doc_line, const RenderContext& ctx) {
    int x = line.indent;

    for (const auto& span : line.spans) {
        // 检查是否需要搜索高亮
        bool has_search_match = (ctx.search && ctx.search->enabled && !ctx.search->matches.empty());

        if (has_search_match) {
            // 按字符渲染以支持部分高亮
            const std::string& text = span.text;
            int char_col = x;

            for (size_t i = 0; i < text.size(); ) {
                // 获取字符宽度（处理UTF-8）
                int char_bytes = 1;
                unsigned char c = text[i];
                if ((c & 0x80) == 0) {
                    char_bytes = 1;
                } else if ((c & 0xE0) == 0xC0) {
                    char_bytes = 2;
                } else if ((c & 0xF0) == 0xE0) {
                    char_bytes = 3;
                } else if ((c & 0xF8) == 0xF0) {
                    char_bytes = 4;
                }

                std::string ch = text.substr(i, char_bytes);
                int char_width = static_cast<int>(Unicode::display_width(ch));

                uint32_t fg = span.fg;
                uint32_t bg = span.bg;
                uint8_t attrs = span.attrs;

                // 检查搜索匹配
                int match_idx = find_match_at(ctx.search, doc_line, char_col);
                if (match_idx >= 0) {
                    // 搜索高亮
                    if (match_idx == ctx.search->current_match_idx) {
                        fg = colors::SEARCH_CURRENT_FG;
                        bg = colors::SEARCH_CURRENT_BG;
                    } else {
                        fg = colors::SEARCH_MATCH_FG;
                        bg = colors::SEARCH_MATCH_BG;
                    }
                    attrs |= ATTR_BOLD;
                } else if (span.link_index >= 0 && span.link_index == ctx.active_link) {
                    // 活跃链接高亮
                    fg = colors::LINK_ACTIVE;
                    attrs |= ATTR_BOLD;
                } else if (span.field_index >= 0 && span.field_index == ctx.active_field) {
                    // 活跃表单字段高亮
                    fg = colors::SEARCH_CURRENT_FG;
                    bg = colors::INPUT_FOCUS;
                    attrs |= ATTR_BOLD;
                }

                buffer_.set_text(char_col, y, ch, fg, bg, attrs);
                char_col += char_width;
                i += char_bytes;
            }
            x = char_col;
        } else {
            // 无搜索匹配时，整体渲染（更高效）
            uint32_t fg = span.fg;
            uint32_t bg = span.bg;
            uint8_t attrs = span.attrs;

            // 高亮活跃链接
            if (span.link_index >= 0 && span.link_index == ctx.active_link) {
                fg = colors::LINK_ACTIVE;
                attrs |= ATTR_BOLD;
            }
            // 高亮活跃表单字段
            else if (span.field_index >= 0 && span.field_index == ctx.active_field) {
                fg = colors::SEARCH_CURRENT_FG;
                bg = colors::INPUT_FOCUS;
                attrs |= ATTR_BOLD;
            }

            buffer_.set_text(x, y, span.text, fg, bg, attrs);
            x += static_cast<int>(span.display_width());
        }
    }
}

} // namespace tut
