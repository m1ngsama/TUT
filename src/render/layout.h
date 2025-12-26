#pragma once

#include "renderer.h"
#include "colors.h"
#include "../dom_tree.h"
#include "../utils/unicode.h"
#include <vector>
#include <string>
#include <memory>

namespace tut {

/**
 * StyledSpan - 带样式的文本片段
 *
 * 表示一段具有统一样式的文本
 */
struct StyledSpan {
    std::string text;
    uint32_t fg = colors::FG_PRIMARY;
    uint32_t bg = colors::BG_PRIMARY;
    uint8_t attrs = ATTR_NONE;
    int link_index = -1;   // -1表示非链接
    int field_index = -1;  // -1表示非表单字段

    size_t display_width() const {
        return Unicode::display_width(text);
    }
};

/**
 * LayoutLine - 布局行
 *
 * 表示一行渲染内容，由多个StyledSpan组成
 */
struct LayoutLine {
    std::vector<StyledSpan> spans;
    int indent = 0;  // 行首缩进（字符数）
    bool is_blank = false;

    size_t total_width() const {
        size_t width = indent;
        for (const auto& span : spans) {
            width += span.display_width();
        }
        return width;
    }
};

/**
 * LayoutBlock - 布局块
 *
 * 表示一个块级元素的布局结果
 * 如段落、标题、列表项等
 */
struct LayoutBlock {
    std::vector<LayoutLine> lines;
    int margin_top = 0;     // 上边距（行数）
    int margin_bottom = 0;  // 下边距（行数）
    ElementType type = ElementType::PARAGRAPH;
};

/**
 * LinkPosition - 链接位置信息
 */
struct LinkPosition {
    int start_line;   // 起始行
    int end_line;     // 结束行（可能跨多行）
};

/**
 * LayoutResult - 布局结果
 *
 * 整个文档的布局结果
 */
struct LayoutResult {
    std::vector<LayoutBlock> blocks;
    int total_lines = 0;  // 总行数（包括边距）
    std::string title;
    std::string url;

    // 链接位置映射 (link_index -> LinkPosition)
    std::vector<LinkPosition> link_positions;

    // 表单字段位置映射 (field_index -> line_number)
    std::vector<int> field_lines;
};

/**
 * LayoutEngine - 布局引擎
 *
 * 将DOM树转换为布局结果
 */
class LayoutEngine {
public:
    explicit LayoutEngine(int viewport_width);

    /**
     * 计算文档布局
     */
    LayoutResult layout(const DocumentTree& doc);

    /**
     * 设置视口宽度
     */
    void set_viewport_width(int width) { viewport_width_ = width; }

private:
    int viewport_width_;
    int content_width_;  // 实际内容宽度（视口宽度减去边距）
    static constexpr int MARGIN_LEFT = 2;
    static constexpr int MARGIN_RIGHT = 2;

    // 布局上下文
    struct Context {
        int list_depth = 0;
        int ordered_list_counter = 0;
        bool in_blockquote = false;
        bool in_pre = false;
    };

    // 布局处理方法
    void layout_node(const DomNode* node, Context& ctx, std::vector<LayoutBlock>& blocks);
    void layout_block_element(const DomNode* node, Context& ctx, std::vector<LayoutBlock>& blocks);
    void layout_form_element(const DomNode* node, Context& ctx, std::vector<LayoutBlock>& blocks);
    void layout_image_element(const DomNode* node, Context& ctx, std::vector<LayoutBlock>& blocks);
    void layout_text(const DomNode* node, Context& ctx, std::vector<StyledSpan>& spans);

    // 收集内联内容
    void collect_inline_content(const DomNode* node, Context& ctx, std::vector<StyledSpan>& spans);

    // 文本换行
    std::vector<LayoutLine> wrap_text(const std::vector<StyledSpan>& spans, int available_width, int indent = 0);

    // 获取元素样式
    uint32_t get_element_fg_color(ElementType type) const;
    uint8_t get_element_attrs(ElementType type) const;

    // 获取列表标记
    std::string get_list_marker(int depth, bool ordered, int counter) const;
};

/**
 * SearchMatch - 搜索匹配信息
 */
struct SearchMatch {
    int line;           // 文档行号
    int start_col;      // 行内起始列
    int length;         // 匹配长度
};

/**
 * SearchContext - 搜索上下文
 */
struct SearchContext {
    std::vector<SearchMatch> matches;
    int current_match_idx = -1;  // 当前高亮的匹配索引
    bool enabled = false;
};

/**
 * RenderContext - 渲染上下文
 */
struct RenderContext {
    int active_link = -1;    // 当前活跃链接索引
    int active_field = -1;   // 当前活跃表单字段索引
    const SearchContext* search = nullptr;  // 搜索上下文
};

/**
 * DocumentRenderer - 文档渲染器
 *
 * 将LayoutResult渲染到FrameBuffer
 */
class DocumentRenderer {
public:
    explicit DocumentRenderer(FrameBuffer& buffer);

    /**
     * 渲染布局结果到缓冲区
     *
     * @param layout 布局结果
     * @param scroll_offset 滚动偏移（行数）
     * @param ctx 渲染上下文
     */
    void render(const LayoutResult& layout, int scroll_offset, const RenderContext& ctx = {});

private:
    FrameBuffer& buffer_;

    void render_line(const LayoutLine& line, int y, int doc_line, const RenderContext& ctx);

    // 检查位置是否在搜索匹配中
    int find_match_at(const SearchContext* search, int doc_line, int col) const;
};

} // namespace tut
