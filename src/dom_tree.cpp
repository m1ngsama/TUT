#include "dom_tree.h"
#include <gumbo.h>
#include <regex>
#include <cctype>
#include <algorithm>
#include <sstream>

// ============================================================================
// DomNode 辅助方法实现
// ============================================================================

bool DomNode::is_block_element() const {
    if (node_type != NodeType::ELEMENT) return false;

    switch (element_type) {
        case ElementType::HEADING1:
        case ElementType::HEADING2:
        case ElementType::HEADING3:
        case ElementType::HEADING4:
        case ElementType::HEADING5:
        case ElementType::HEADING6:
        case ElementType::PARAGRAPH:
        case ElementType::LIST_ITEM:
        case ElementType::ORDERED_LIST_ITEM:
        case ElementType::BLOCKQUOTE:
        case ElementType::CODE_BLOCK:
        case ElementType::HORIZONTAL_RULE:
        case ElementType::TABLE:
        case ElementType::SECTION_START:
        case ElementType::SECTION_END:
        case ElementType::NAV_START:
        case ElementType::NAV_END:
        case ElementType::HEADER_START:
        case ElementType::HEADER_END:
        case ElementType::ASIDE_START:
        case ElementType::ASIDE_END:
        case ElementType::FORM:
            return true;
        default:
            // 通过标签名判断
            return tag_name == "div" || tag_name == "section" ||
                   tag_name == "article" || tag_name == "main" ||
                   tag_name == "header" || tag_name == "footer" ||
                   tag_name == "nav" || tag_name == "aside" ||
                   tag_name == "ul" || tag_name == "ol" ||
                   tag_name == "li" || tag_name == "dl" ||
                   tag_name == "dt" || tag_name == "dd" ||
                   tag_name == "pre" || tag_name == "hr" ||
                   tag_name == "table" || tag_name == "tr" ||
                   tag_name == "th" || tag_name == "td" ||
                   tag_name == "form" || tag_name == "fieldset";
    }
}

bool DomNode::is_inline_element() const {
    if (node_type != NodeType::ELEMENT) return false;

    switch (element_type) {
        case ElementType::LINK:
        case ElementType::TEXT:
        case ElementType::INPUT:
        case ElementType::TEXTAREA:
        case ElementType::SELECT:
        case ElementType::BUTTON:
        case ElementType::OPTION:
            return true;
        default:
            // 通过标签名判断常见的内联元素
            return tag_name == "a" || tag_name == "span" ||
                   tag_name == "strong" || tag_name == "b" ||
                   tag_name == "em" || tag_name == "i" ||
                   tag_name == "code" || tag_name == "kbd" ||
                   tag_name == "mark" || tag_name == "small" ||
                   tag_name == "sub" || tag_name == "sup" ||
                   tag_name == "u" || tag_name == "abbr" ||
                   tag_name == "cite" || tag_name == "q" ||
                   tag_name == "label";
    }
}

bool DomNode::should_render() const {
    // 过滤不应该渲染的元素
    if (tag_name == "script" || tag_name == "style" ||
        tag_name == "noscript" || tag_name == "template" ||
        (tag_name == "input" && input_type == "hidden")) {
        return false;
    }
    return true;
}

std::string DomNode::get_all_text() const {
    std::string result;

    if (node_type == NodeType::TEXT) {
        result = text_content;
    } else {
        // Special handling for form elements to return their value/placeholder for representation
        if (element_type == ElementType::INPUT) {
            // For inputs, we might want to return nothing here as they are rendered specially,
            // or return their value. For simple text extraction, maybe empty is better.
        } else if (element_type == ElementType::TEXTAREA) {
             for (const auto& child : children) {
                result += child->get_all_text();
            }
        } else {
            for (const auto& child : children) {
                result += child->get_all_text();
            }
        }
    }

    return result;
}

// ============================================================================
// DomTreeBuilder 实现
// ============================================================================

// Add a member to track current form ID
namespace { 
    int g_current_form_id = -1;
    int g_next_form_id = 0;
}

DomTreeBuilder::DomTreeBuilder() = default;
DomTreeBuilder::~DomTreeBuilder() = default;

DocumentTree DomTreeBuilder::build(const std::string& html, const std::string& base_url) {
    // Reset form tracking
    g_current_form_id = -1;
    g_next_form_id = 0;

    // 1. 使用gumbo解析HTML
    GumboOutput* output = gumbo_parse(html.c_str());

    // 2. 转换为DomNode树
    DocumentTree tree;
    tree.url = base_url;
    tree.root = convert_node(output->root, tree.links, tree.form_fields, base_url);

    // 3. 提取标题
    if (tree.root) {
        tree.title = extract_title(tree.root.get());
    }

    // 4. 清理gumbo资源
    gumbo_destroy_output(&kGumboDefaultOptions, output);

    return tree;
}

std::unique_ptr<DomNode> DomTreeBuilder::convert_node(
    GumboNode* gumbo_node,
    std::vector<Link>& links,
    std::vector<DomNode*>& form_fields,
    const std::string& base_url
) {
    if (!gumbo_node) return nullptr;

    auto node = std::make_unique<DomNode>();

    if (gumbo_node->type == GUMBO_NODE_ELEMENT) {
        node->node_type = NodeType::ELEMENT;
        GumboElement& element = gumbo_node->v.element;

        // 设置标签名
        node->tag_name = gumbo_normalized_tagname(element.tag);
        node->element_type = map_gumbo_tag_to_element_type(element.tag);

        // Assign current form ID to children
        node->form_id = g_current_form_id;

        // Special handling for FORM tag
        if (element.tag == GUMBO_TAG_FORM) {
            node->form_id = g_next_form_id++;
            g_current_form_id = node->form_id;

            GumboAttribute* action_attr = gumbo_get_attribute(&element.attributes, "action");
            if (action_attr) node->action = resolve_url(action_attr->value, base_url);
            else node->action = base_url; // Default to current URL

            GumboAttribute* method_attr = gumbo_get_attribute(&element.attributes, "method");
            if (method_attr) node->method = method_attr->value;
            else node->method = "GET";
            
            // Transform to uppercase
            std::transform(node->method.begin(), node->method.end(), node->method.begin(), ::toupper);
        }

        // Handle INPUT
        if (element.tag == GUMBO_TAG_INPUT) {
             GumboAttribute* type_attr = gumbo_get_attribute(&element.attributes, "type");
             node->input_type = type_attr ? type_attr->value : "text";

             GumboAttribute* name_attr = gumbo_get_attribute(&element.attributes, "name");
             if (name_attr) node->name = name_attr->value;

             GumboAttribute* value_attr = gumbo_get_attribute(&element.attributes, "value");
             if (value_attr) node->value = value_attr->value;
             
             GumboAttribute* placeholder_attr = gumbo_get_attribute(&element.attributes, "placeholder");
             if (placeholder_attr) node->placeholder = placeholder_attr->value;

             if (gumbo_get_attribute(&element.attributes, "checked")) {
                 node->checked = true;
             }

             // Register form field
             if (node->input_type != "hidden") {
                 node->field_index = form_fields.size();
                 form_fields.push_back(node.get());
             }
        }
        
        // Handle TEXTAREA
        if (element.tag == GUMBO_TAG_TEXTAREA) {
             node->input_type = "textarea";
             GumboAttribute* name_attr = gumbo_get_attribute(&element.attributes, "name");
             if (name_attr) node->name = name_attr->value;
             
             GumboAttribute* placeholder_attr = gumbo_get_attribute(&element.attributes, "placeholder");
             if (placeholder_attr) node->placeholder = placeholder_attr->value;
             
             // Register form field
             node->field_index = form_fields.size();
             form_fields.push_back(node.get());
        }
        
        // Handle SELECT
        if (element.tag == GUMBO_TAG_SELECT) {
             node->input_type = "select";
             GumboAttribute* name_attr = gumbo_get_attribute(&element.attributes, "name");
             if (name_attr) node->name = name_attr->value;

             // Register form field
             node->field_index = form_fields.size();
             form_fields.push_back(node.get());
        }

        // Handle OPTION
        if (element.tag == GUMBO_TAG_OPTION) {
             node->input_type = "option";
             GumboAttribute* value_attr = gumbo_get_attribute(&element.attributes, "value");
             if (value_attr) node->value = value_attr->value;
             if (gumbo_get_attribute(&element.attributes, "selected")) {
                 node->checked = true;
             }
        }
        
        // Handle BUTTON
        if (element.tag == GUMBO_TAG_BUTTON) {
             GumboAttribute* type_attr = gumbo_get_attribute(&element.attributes, "type");
             node->input_type = type_attr ? type_attr->value : "submit";
             
             GumboAttribute* name_attr = gumbo_get_attribute(&element.attributes, "name");
             if (name_attr) node->name = name_attr->value;
             
             GumboAttribute* value_attr = gumbo_get_attribute(&element.attributes, "value");
             if (value_attr) node->value = value_attr->value;

             // Register form field
             node->field_index = form_fields.size();
             form_fields.push_back(node.get());
        }

        // Handle IMG
        if (element.tag == GUMBO_TAG_IMG) {
            GumboAttribute* src_attr = gumbo_get_attribute(&element.attributes, "src");
            if (src_attr && src_attr->value) {
                node->img_src = resolve_url(src_attr->value, base_url);
            }

            GumboAttribute* alt_attr = gumbo_get_attribute(&element.attributes, "alt");
            if (alt_attr) node->alt_text = alt_attr->value;

            GumboAttribute* width_attr = gumbo_get_attribute(&element.attributes, "width");
            if (width_attr && width_attr->value) {
                try { node->img_width = std::stoi(width_attr->value); } catch (...) {}
            }

            GumboAttribute* height_attr = gumbo_get_attribute(&element.attributes, "height");
            if (height_attr && height_attr->value) {
                try { node->img_height = std::stoi(height_attr->value); } catch (...) {}
            }
        }


        // 处理<a>标签
        if (element.tag == GUMBO_TAG_A) {
            GumboAttribute* href_attr = gumbo_get_attribute(&element.attributes, "href");
            if (href_attr && href_attr->value) {
                std::string href = href_attr->value;
                // 过滤锚点链接和javascript链接
                if (!href.empty() && href[0] != '#' &&
                    href.find("javascript:") != 0 &&
                    href.find("mailto:") != 0) {

                    node->href = resolve_url(href, base_url);

                    // 注册到全局链接列表
                    Link link;
                    link.text = extract_text_from_gumbo(gumbo_node);
                    link.url = node->href;
                    link.position = links.size();

                    links.push_back(link);
                    node->link_index = links.size() - 1;
                    node->element_type = ElementType::LINK;
                }
            }
        }

        // 处理表格单元格属性
        if (element.tag == GUMBO_TAG_TH) {
            node->is_table_header = true;
        }

        if (element.tag == GUMBO_TAG_TD || element.tag == GUMBO_TAG_TH) {
            GumboAttribute* colspan_attr = gumbo_get_attribute(&element.attributes, "colspan");
            if (colspan_attr && colspan_attr->value) {
                node->colspan = std::stoi(colspan_attr->value);
            }

            GumboAttribute* rowspan_attr = gumbo_get_attribute(&element.attributes, "rowspan");
            if (rowspan_attr && rowspan_attr->value) {
                node->rowspan = std::stoi(rowspan_attr->value);
            }
        }

        // 递归处理子节点
        GumboVector* children = &element.children;
        for (unsigned int i = 0; i < children->length; ++i) {
            auto child = convert_node(
                static_cast<GumboNode*>(children->data[i]),
                links,
                form_fields,
                base_url
            );
            if (child) {
                child->parent = node.get();
                node->children.push_back(std::move(child));
                
                // For TEXTAREA, content is value
                if (element.tag == GUMBO_TAG_TEXTAREA && child->node_type == NodeType::TEXT) {
                    node->value += child->text_content;
                }
            }
        }
        
        // Reset form ID if we are exiting a form
        if (element.tag == GUMBO_TAG_FORM) {
            g_current_form_id = -1; // Assuming no nested forms
        }
    }
    else if (gumbo_node->type == GUMBO_NODE_TEXT) {
        node->node_type = NodeType::TEXT;
        std::string text = gumbo_node->v.text.text;

        // 解码HTML实体
        node->text_content = decode_html_entities(text);
        node->form_id = g_current_form_id;
    }
    else if (gumbo_node->type == GUMBO_NODE_DOCUMENT) {
        node->node_type = NodeType::DOCUMENT;
        node->tag_name = "document";

        // 处理文档节点的子节点
        GumboDocument& doc = gumbo_node->v.document;
        for (unsigned int i = 0; i < doc.children.length; ++i) {
            auto child = convert_node(
                static_cast<GumboNode*>(doc.children.data[i]),
                links,
                form_fields,
                base_url
            );
            if (child) {
                child->parent = node.get();
                node->children.push_back(std::move(child));
            }
        }
    }

    return node;
}

std::string DomTreeBuilder::extract_title(DomNode* root) {
    if (!root) return "";

    // 递归查找<title>标签
    std::function<std::string(DomNode*)> find_title = [&](DomNode* node) -> std::string {
        if (!node) return "";

        if (node->tag_name == "title") {
            return node->get_all_text();
        }

        for (auto& child : node->children) {
            std::string title = find_title(child.get());
            if (!title.empty()) return title;
        }

        return "";
    };

    std::string title = find_title(root);

    // 如果没有<title>，尝试找第一个<h1>
    if (title.empty()) {
        std::function<std::string(DomNode*)> find_h1 = [&](DomNode* node) -> std::string {
            if (!node) return "";

            if (node->tag_name == "h1") {
                return node->get_all_text();
            }

            for (auto& child : node->children) {
                std::string h1 = find_h1(child.get());
                if (!h1.empty()) return h1;
            }

            return "";
        };

        title = find_h1(root);
    }

    // 清理标题中的多余空白
    title = std::regex_replace(title, std::regex(R"(\s+)"), " ");

    // 去除首尾空白
    size_t start = title.find_first_not_of(" \t\n\r");
    if (start == std::string::npos) return "";

    size_t end = title.find_last_not_of(" \t\n\r");
    return title.substr(start, end - start + 1);
}

std::string DomTreeBuilder::extract_text_from_gumbo(GumboNode* node) {
    if (!node) return "";

    std::string text;

    if (node->type == GUMBO_NODE_TEXT) {
        text = node->v.text.text;
    } else if (node->type == GUMBO_NODE_ELEMENT) {
        GumboVector* children = &node->v.element.children;
        for (unsigned int i = 0; i < children->length; ++i) {
            text += extract_text_from_gumbo(static_cast<GumboNode*>(children->data[i]));
        }
    }

    return text;
}

ElementType DomTreeBuilder::map_gumbo_tag_to_element_type(int gumbo_tag) {
    switch (gumbo_tag) {
        case GUMBO_TAG_H1: return ElementType::HEADING1;
        case GUMBO_TAG_H2: return ElementType::HEADING2;
        case GUMBO_TAG_H3: return ElementType::HEADING3;
        case GUMBO_TAG_H4: return ElementType::HEADING4;
        case GUMBO_TAG_H5: return ElementType::HEADING5;
        case GUMBO_TAG_H6: return ElementType::HEADING6;
        case GUMBO_TAG_P: return ElementType::PARAGRAPH;
        case GUMBO_TAG_A: return ElementType::LINK;
        case GUMBO_TAG_LI: return ElementType::LIST_ITEM;
        case GUMBO_TAG_BLOCKQUOTE: return ElementType::BLOCKQUOTE;
        case GUMBO_TAG_PRE: return ElementType::CODE_BLOCK;
        case GUMBO_TAG_HR: return ElementType::HORIZONTAL_RULE;
        case GUMBO_TAG_BR: return ElementType::LINE_BREAK;
        case GUMBO_TAG_TABLE: return ElementType::TABLE;
        case GUMBO_TAG_IMG: return ElementType::IMAGE;
        case GUMBO_TAG_FORM: return ElementType::FORM;
        case GUMBO_TAG_INPUT: return ElementType::INPUT;
        case GUMBO_TAG_TEXTAREA: return ElementType::TEXTAREA;
        case GUMBO_TAG_SELECT: return ElementType::SELECT;
        case GUMBO_TAG_OPTION: return ElementType::OPTION;
        case GUMBO_TAG_BUTTON: return ElementType::BUTTON;
        default: return ElementType::TEXT;
    }
}

std::string DomTreeBuilder::resolve_url(const std::string& url, const std::string& base_url) {
    if (url.empty()) return "";

    // 绝对URL（http://或https://）
    if (url.find("http://") == 0 || url.find("https://") == 0) {
        return url;
    }

    // 协议相对URL（//example.com）
    if (url.size() >= 2 && url[0] == '/' && url[1] == '/') {
        // 从base_url提取协议
        size_t proto_end = base_url.find("://");
        if (proto_end != std::string::npos) {
            return base_url.substr(0, proto_end) + ":" + url;
        }
        return "https:" + url;
    }

    if (base_url.empty()) return url;

    // 绝对路径（/path）
    if (url[0] == '/') {
        // 提取base_url的scheme和host
        size_t proto_end = base_url.find("://");
        if (proto_end == std::string::npos) return url;

        size_t host_start = proto_end + 3;
        size_t path_start = base_url.find('/', host_start);

        std::string base_origin;
        if (path_start != std::string::npos) {
            base_origin = base_url.substr(0, path_start);
        } else {
            base_origin = base_url;
        }

        return base_origin + url;
    }

    // 相对路径（relative/path）
    // 找到base_url的路径部分
    size_t proto_end = base_url.find("://");
    if (proto_end == std::string::npos) return url;

    size_t host_start = proto_end + 3;
    size_t path_start = base_url.find('/', host_start);

    std::string base_path;
    if (path_start != std::string::npos) {
        // 找到最后一个/
        size_t last_slash = base_url.rfind('/');
        if (last_slash != std::string::npos) {
            base_path = base_url.substr(0, last_slash + 1);
        } else {
            base_path = base_url + "/";
        }
    } else {
        base_path = base_url + "/";
    }

    return base_path + url;
}

const std::map<std::string, std::string>& DomTreeBuilder::get_entity_map() {
    static std::map<std::string, std::string> entity_map = {
        {"&nbsp;", " "}, {"&lt;", "<"}, {"&gt;", ">"},
        {"&amp;", "&"}, {"&quot;", "\""}, {"&apos;", "'"},
        {"&copy;", "©"}, {"&reg;", "®"}, {"&trade;", "™"},
        {"&euro;", "€"}, {"&pound;", "£"}, {"&yen;", "¥"},
        {"&cent;", "¢"}, {"&sect;", "§"}, {"&para;", "¶"},
        {"&dagger;", "†"}, {"&Dagger;", "‡"}, {"&bull;", "•"},
        {"&hellip;", "…"}, {"&prime;", "′"}, {"&Prime;", "″"},
        {"&lsaquo;", "‹"}, {"&rsaquo;", "›"}, {"&laquo;", "«"},
        {"&raquo;", "»"}, {"&lsquo;", "'"}, {"&rsquo;", "'"},
        {"&ldquo;", "\u201C"}, {"&rdquo;", "\u201D"}, {"&mdash;", "—"},
        {"&ndash;", "–"}, {"&iexcl;", "¡"}, {"&iquest;", "¿"},
        {"&times;", "×"}, {"&divide;", "÷"}, {"&plusmn;", "±"},
        {"&deg;", "°"}, {"&micro;", "µ"}, {"&middot;", "·"},
        {"&frac14;", "¼"}, {"&frac12;", "½"}, {"&frac34;", "¾"},
        {"&sup1;", "¹"}, {"&sup2;", "²"}, {"&sup3;", "³"},
        {"&alpha;", "α"}, {"&beta;", "β"}, {"&gamma;", "γ"},
        {"&delta;", "δ"}, {"&epsilon;", "ε"}, {"&theta;", "θ"},
        {"&lambda;", "λ"}, {"&mu;", "μ"}, {"&pi;", "π"},
        {"&sigma;", "σ"}, {"&tau;", "τ"}, {"&phi;", "φ"},
        {"&omega;", "ω"}
    };
    return entity_map;
}

std::string DomTreeBuilder::decode_html_entities(const std::string& text) {
    std::string result = text;
    const auto& entity_map = get_entity_map();

    // 替换命名实体
    for (const auto& [entity, replacement] : entity_map) {
        size_t pos = 0;
        while ((pos = result.find(entity, pos)) != std::string::npos) {
            result.replace(pos, entity.length(), replacement);
            pos += replacement.length();
        }
    }

    // 替换数字实体 &#123; 或 &#xAB;
    std::regex numeric_entity(R"(&#(\d+);)");
    std::regex hex_entity(R"(&#x([0-9A-Fa-f]+);)");

    // 处理十进制数字实体
    std::string temp;
    size_t last_pos = 0;
    std::smatch match;
    std::string::const_iterator search_start(result.cbegin());

    while (std::regex_search(search_start, result.cend(), match, numeric_entity)) {
        size_t match_pos = match.position() + std::distance(result.cbegin(), search_start);
        temp += result.substr(last_pos, match_pos - last_pos);

        int code = std::stoi(match[1].str());
        if (code > 0 && code < 0x110000) {
            // 简单的UTF-8编码（仅支持基本多文种平面）
            if (code < 0x80) {
                temp += static_cast<char>(code);
            } else if (code < 0x800) {
                temp += static_cast<char>(0xC0 | (code >> 6));
                temp += static_cast<char>(0x80 | (code & 0x3F));
            } else if (code < 0x10000) {
                temp += static_cast<char>(0xE0 | (code >> 12));
                temp += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
                temp += static_cast<char>(0x80 | (code & 0x3F));
            } else {
                temp += static_cast<char>(0xF0 | (code >> 18));
                temp += static_cast<char>(0x80 | ((code >> 12) & 0x3F));
                temp += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
                temp += static_cast<char>(0x80 | (code & 0x3F));
            }
        }

        last_pos = match_pos + match[0].length();
        search_start = result.cbegin() + last_pos;
    }
    temp += result.substr(last_pos);
    result = temp;

    // 处理十六进制数字实体
    temp.clear();
    last_pos = 0;
    search_start = result.cbegin();

    while (std::regex_search(search_start, result.cend(), match, hex_entity)) {
        size_t match_pos = match.position() + std::distance(result.cbegin(), search_start);
        temp += result.substr(last_pos, match_pos - last_pos);

        int code = std::stoi(match[1].str(), nullptr, 16);
        if (code > 0 && code < 0x110000) {
            if (code < 0x80) {
                temp += static_cast<char>(code);
            } else if (code < 0x800) {
                temp += static_cast<char>(0xC0 | (code >> 6));
                temp += static_cast<char>(0x80 | (code & 0x3F));
            } else if (code < 0x10000) {
                temp += static_cast<char>(0xE0 | (code >> 12));
                temp += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
                temp += static_cast<char>(0x80 | (code & 0x3F));
            } else {
                temp += static_cast<char>(0xF0 | (code >> 18));
                temp += static_cast<char>(0x80 | ((code >> 12) & 0x3F));
                temp += static_cast<char>(0x80 | ((code >> 6) & 0x3F));
                temp += static_cast<char>(0x80 | (code & 0x3F));
            }
        }

        last_pos = match_pos + match[0].length();
        search_start = result.cbegin() + last_pos;
    }
    temp += result.substr(last_pos);

    return temp;
}
