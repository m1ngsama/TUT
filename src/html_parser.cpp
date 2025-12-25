#include "html_parser.h"
#include "dom_tree.h"
#include <stdexcept>

// ============================================================================
// HtmlParser::Impl 实现
// ============================================================================

class HtmlParser::Impl {
public:
    bool keep_code_blocks = true;
    bool keep_lists = true;

    DomTreeBuilder tree_builder;

    DocumentTree parse_tree(const std::string& html, const std::string& base_url) {
        return tree_builder.build(html, base_url);
    }

    // 将DocumentTree转换为ParsedDocument（向后兼容）
    ParsedDocument convert_to_parsed_document(const DocumentTree& tree) {
        ParsedDocument doc;
        doc.title = tree.title;
        doc.url = tree.url;
        doc.links = tree.links;

        // 递归遍历DOM树，收集ContentElement
        if (tree.root) {
            collect_content_elements(tree.root.get(), doc.elements);
        }

        return doc;
    }

private:
    void collect_content_elements(DomNode* node, std::vector<ContentElement>& elements) {
        if (!node || !node->should_render()) return;

        if (node->node_type == NodeType::ELEMENT) {
            ContentElement elem;
            elem.type = node->element_type;
            elem.url = node->href;
            elem.level = 0;  // TODO: 根据需要计算层级
            elem.list_number = 0;
            elem.nesting_level = 0;

            // 提取文本内容
            elem.text = node->get_all_text();

            // 收集内联链接
            collect_inline_links(node, elem.inline_links);

            // 只添加有内容的元素
            if (!elem.text.empty() || node->element_type == ElementType::HORIZONTAL_RULE) {
                elements.push_back(elem);
            }
        }

        // 递归处理子节点
        for (const auto& child : node->children) {
            collect_content_elements(child.get(), elements);
        }
    }

    void collect_inline_links(DomNode* node, std::vector<InlineLink>& links) {
        if (!node) return;

        if (node->element_type == ElementType::LINK && node->link_index >= 0) {
            InlineLink link;
            link.text = node->get_all_text();
            link.url = node->href;
            link.link_index = node->link_index;
            link.start_pos = 0;  // 简化：不计算精确位置
            link.end_pos = link.text.length();
            links.push_back(link);
        }

        for (const auto& child : node->children) {
            collect_inline_links(child.get(), links);
        }
    }
};

// ============================================================================
// HtmlParser 公共接口实现
// ============================================================================

HtmlParser::HtmlParser() : pImpl(std::make_unique<Impl>()) {}

HtmlParser::~HtmlParser() = default;

DocumentTree HtmlParser::parse_tree(const std::string& html, const std::string& base_url) {
    return pImpl->parse_tree(html, base_url);
}

ParsedDocument HtmlParser::parse(const std::string& html, const std::string& base_url) {
    // 使用新的DOM树解析，然后转换为旧格式
    DocumentTree tree = pImpl->parse_tree(html, base_url);
    return pImpl->convert_to_parsed_document(tree);
}

void HtmlParser::set_keep_code_blocks(bool keep) {
    pImpl->keep_code_blocks = keep;
}

void HtmlParser::set_keep_lists(bool keep) {
    pImpl->keep_lists = keep;
}
