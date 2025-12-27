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

    // 处理块级元素
    if (node->is_block_element()) {
        layout_block_element(node, ctx, blocks);
        return;
    }

    // 处理链接 - 当链接单独出现时（不在段落内），创建一个单独的块
    if (node->element_type == ElementType::LINK && node->link_index >= 0) {
        // 检查链接是否有可见文本
        std::string link_text = node->get_all_text();
        // 去除空白
        size_t start = link_text.find_first_not_of(" \t\n\r");
        size_t end = link_text.find_last_not_of(" \t\n\r");
        if (start != std::string::npos && end != std::string::npos) {
            link_text = link_text.substr(start, end - start + 1);
        } else {
            link_text = "";
        }

        if (!link_text.empty()) {
            LayoutBlock block;
            block.type = ElementType::PARAGRAPH;
            block.margin_top = 0;
            block.margin_bottom = 0;

            LayoutLine line;
            line.indent = MARGIN_LEFT;

            StyledSpan span;
            span.text = link_text;
            span.fg = colors::LINK_FG;
            span.attrs = ATTR_UNDERLINE;
            span.link_index = node->link_index;
            line.spans.push_back(span);

            block.lines.push_back(line);
            blocks.push_back(block);
        }
        return;
    }

    // 处理容器元素 - 递归处理子节点
    // 这包括：html, body, div, table, span, center 等所有容器类元素
    if (node->node_type == NodeType::ELEMENT && !node->children.empty()) {
        for (const auto& child : node->children) {
            layout_node(child.get(), ctx, blocks);
        }
        return;
    }

    // 处理独立文本节点
    if (node->node_type == NodeType::TEXT && !node->text_content.empty()) {
        std::string text = node->text_content;
        // 去除首尾空白
        size_t start = text.find_first_not_of(" \t\n\r");
        size_t end = text.find_last_not_of(" \t\n\r");
        if (start != std::string::npos && end != std::string::npos) {
            text = text.substr(start, end - start + 1);
        } else {
            return; // 空白文本，跳过
        }

        if (!text.empty()) {
            LayoutBlock block;
            block.type = ElementType::TEXT;
            block.margin_top = 0;
            block.margin_bottom = 0;

            std::vector<StyledSpan> spans;
            StyledSpan span;
            span.text = text;
            span.fg = colors::FG_PRIMARY;
            spans.push_back(span);

            block.lines = wrap_text(spans, content_width_, MARGIN_LEFT);
            if (!block.lines.empty()) {
                blocks.push_back(block);
            }
        }
    }
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
        // 下拉选择 - 显示当前选中的选项
        StyledSpan span;
        std::string selected_text = "Select";
        if (node->selected_option >= 0 && node->selected_option < static_cast<int>(node->options.size())) {
            selected_text = node->options[node->selected_option].second;
        }
        span.text = "[▼ " + selected_text + "]";
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

    // 检查是否有解码后的图片数据
    if (node->image_data.is_valid()) {
        // 渲染 ASCII Art
        ImageRenderer renderer;
        renderer.set_mode(ImageRenderer::Mode::BLOCKS);
        renderer.set_color_enabled(true);

        // 计算图片最大尺寸（留出左边距）
        int max_width = content_width_;
        int max_height = 30;  // 限制高度

        // 如果节点指定了尺寸，使用更小的值
        if (node->img_width > 0) {
            max_width = std::min(max_width, node->img_width);
        }
        if (node->img_height > 0) {
            max_height = std::min(max_height, node->img_height / 2);  // 考虑字符高宽比
        }

        AsciiImage ascii = renderer.render(node->image_data, max_width, max_height);

        if (!ascii.lines.empty()) {
            for (size_t i = 0; i < ascii.lines.size(); ++i) {
                LayoutLine line;
                line.indent = MARGIN_LEFT;

                // 将每一行作为一个 span
                // 但由于颜色可能不同，需要逐字符处理
                const std::string& line_text = ascii.lines[i];
                const std::vector<uint32_t>& line_colors = ascii.colors[i];

                // 为了效率，尝试合并相同颜色的字符
                size_t pos = 0;
                while (pos < line_text.size()) {
                    // 获取当前字符的字节数（UTF-8）
                    int char_bytes = 1;
                    unsigned char c = line_text[pos];
                    if ((c & 0x80) == 0) {
                        char_bytes = 1;
                    } else if ((c & 0xE0) == 0xC0) {
                        char_bytes = 2;
                    } else if ((c & 0xF0) == 0xE0) {
                        char_bytes = 3;
                    } else if ((c & 0xF8) == 0xF0) {
                        char_bytes = 4;
                    }

                    // 获取颜色索引（基于显示宽度位置）
                    size_t color_idx = 0;
                    for (size_t j = 0; j < pos; ) {
                        unsigned char ch = line_text[j];
                        int bytes = 1;
                        if ((ch & 0x80) == 0) bytes = 1;
                        else if ((ch & 0xE0) == 0xC0) bytes = 2;
                        else if ((ch & 0xF0) == 0xE0) bytes = 3;
                        else if ((ch & 0xF8) == 0xF0) bytes = 4;
                        color_idx++;
                        j += bytes;
                    }

                    uint32_t color = (color_idx < line_colors.size()) ? line_colors[color_idx] : colors::FG_PRIMARY;

                    StyledSpan span;
                    span.text = line_text.substr(pos, char_bytes);
                    span.fg = color;
                    span.attrs = ATTR_NONE;
                    line.spans.push_back(span);

                    pos += char_bytes;
                }

                block.lines.push_back(line);
            }
            blocks.push_back(block);
            return;
        }
    }

    // 回退到占位符
    LayoutLine line;
    line.indent = MARGIN_LEFT;

    std::string placeholder = make_image_placeholder(node->alt_text, node->img_src);

    StyledSpan span;
    span.text = placeholder;
    span.fg = colors::FG_DIM;  // 使用较暗的颜色表示占位符
    span.attrs = ATTR_NONE;

    line.spans.push_back(span);
    block.lines.push_back(line);
    blocks.push_back(block);
}

// 辅助函数：检查是否需要在两个文本之间添加空格
static bool needs_space_between(const std::string& prev, const std::string& next) {
    if (prev.empty() || next.empty()) return false;

    char last_char = prev.back();
    char first_char = next.front();

    // 检查前一个是否以空白结尾
    bool prev_ends_with_space = (last_char == ' ' || last_char == '\t' ||
                                  last_char == '\n' || last_char == '\r');
    if (prev_ends_with_space) return false;

    // 检查当前是否以空白开头
    bool curr_starts_with_space = (first_char == ' ' || first_char == '\t' ||
                                    first_char == '\n' || first_char == '\r');
    if (curr_starts_with_space) return false;

    // 检查是否是标点符号（不需要空格）
    bool is_punct = (first_char == '.' || first_char == ',' ||
                     first_char == '!' || first_char == '?' ||
                     first_char == ':' || first_char == ';' ||
                     first_char == ')' || first_char == ']' ||
                     first_char == '}' || first_char == '|' ||
                     first_char == '\'' || first_char == '"');
    if (is_punct) return false;

    // 检查前一个字符是否是特殊符号（不需要空格）
    bool prev_is_open = (last_char == '(' || last_char == '[' ||
                         last_char == '{' || last_char == '\'' ||
                         last_char == '"');
    if (prev_is_open) return false;

    return true;
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
        for (size_t i = 0; i < node->children.size(); ++i) {
            const auto& child = node->children[i];

            if (child->node_type == NodeType::TEXT) {
                std::string text = child->text_content;

                // 检查是否需要在之前的内容和当前内容之间添加空格
                if (!spans.empty() && !text.empty()) {
                    if (needs_space_between(spans.back().text, text)) {
                        spans.back().text += " ";
                    }
                }

                StyledSpan span;
                span.text = text;
                span.fg = fg;
                span.attrs = attrs;
                span.link_index = link_idx;

                if (ctx.in_blockquote) {
                    span.fg = colors::QUOTE_FG;
                }

                spans.push_back(span);
            } else if (!child->is_block_element()) {
                // 获取子节点的全部文本，用于检查是否需要空格
                std::string child_text = child->get_all_text();

                // 在递归调用前检查空格
                if (!spans.empty() && !child_text.empty()) {
                    if (needs_space_between(spans.back().text, child_text)) {
                        spans.back().text += " ";
                    }
                }

                collect_inline_content(child.get(), ctx, spans);
            }
        }
    }
}

void LayoutEngine::layout_text(const DomNode* node, Context& ctx, std::vector<StyledSpan>& spans) {
    if (!node || node->text_content.empty()) return;

    std::string text = node->text_content;

    // 检查是否需要在之前的内容和当前内容之间添加空格
    if (!spans.empty() && !text.empty()) {
        if (needs_space_between(spans.back().text, text)) {
            spans.back().text += " ";
        }
    }

    StyledSpan span;
    span.text = text;
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
    bool is_line_start = true;  // 整行的开始标记

    for (const auto& span : spans) {
        // 分词处理
        std::istringstream iss(span.text);
        std::string word;
        bool first_word_in_span = true;

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
                is_line_start = true;
            }

            // 添加空格（如果不是行首且不是第一个单词）
            // 需要在不同 span 之间也添加空格
            if (!is_line_start) {
                // 检查是否需要空格（避免在标点前加空格）
                char first_char = word.front();
                bool is_punct = (first_char == '.' || first_char == ',' ||
                                 first_char == '!' || first_char == '?' ||
                                 first_char == ':' || first_char == ';' ||
                                 first_char == ')' || first_char == ']' ||
                                 first_char == '}' || first_char == '|');
                if (!is_punct && !current_line.spans.empty()) {
                    current_line.spans.back().text += " ";
                    current_width += 1;
                }
            }

            // 添加单词
            StyledSpan word_span = span;
            word_span.text = word;
            current_line.spans.push_back(word_span);
            current_width += word_width;
            is_line_start = false;
            first_word_in_span = false;
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
