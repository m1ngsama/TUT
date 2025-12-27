#pragma once

#include "html_parser.h"
#include "render/image.h"
#include <string>
#include <vector>
#include <memory>
#include <map>

// Forward declaration for gumbo
struct GumboInternalNode;
struct GumboInternalOutput;
typedef struct GumboInternalNode GumboNode;
typedef struct GumboInternalOutput GumboOutput;

// DOM节点类型
enum class NodeType {
    ELEMENT,    // 元素节点（h1, p, div等）
    TEXT,       // 文本节点
    DOCUMENT    // 文档根节点
};

// DOM节点结构
struct DomNode {
    NodeType node_type;
    ElementType element_type;  // 复用现有的ElementType
    std::string tag_name;      // "div", "p", "h1"等
    std::string text_content;  // TEXT节点的文本内容

    // 树结构
    std::vector<std::unique_ptr<DomNode>> children;
    DomNode* parent = nullptr;  // 非拥有指针

    // 链接属性
    std::string href;
    int link_index = -1;  // -1表示非链接
    int field_index = -1; // -1表示非表单字段

    // 图片属性
    std::string img_src;   // 图片URL
    std::string alt_text;  // 图片alt文本
    int img_width = -1;    // 图片宽度 (-1表示未指定)
    int img_height = -1;   // 图片高度 (-1表示未指定)
    tut::ImageData image_data;  // 解码后的图片数据

    // 表格属性
    bool is_table_header = false;
    int colspan = 1;
    int rowspan = 1;

    // 表单属性
    std::string action;
    std::string method;
    std::string name;
    std::string value;
    std::string input_type; // text, password, checkbox, radio, submit, hidden
    std::string placeholder;
    bool checked = false;
    int form_id = -1;

    // SELECT元素的选项
    std::vector<std::pair<std::string, std::string>> options;  // (value, text) pairs
    int selected_option = 0;  // 当前选中的选项索引

    // 辅助方法
    bool is_block_element() const;
    bool is_inline_element() const;
    bool should_render() const;  // 是否应该渲染（过滤script、style等）
    std::string get_all_text() const;  // 递归获取所有文本内容
};

// 文档树结构
struct DocumentTree {
    std::unique_ptr<DomNode> root;
    std::vector<Link> links;  // 全局链接列表
    std::vector<DomNode*> form_fields; // 全局表单字段列表 (非拥有指针)
    std::vector<DomNode*> images;  // 全局图片列表 (非拥有指针)
    std::string title;
    std::string url;
};

// DOM树构建器
class DomTreeBuilder {
public:
    DomTreeBuilder();
    ~DomTreeBuilder();

    // 从HTML构建DOM树
    DocumentTree build(const std::string& html, const std::string& base_url);

private:
    // 将GumboNode转换为DomNode
    std::unique_ptr<DomNode> convert_node(
        GumboNode* gumbo_node,
        std::vector<Link>& links,
        std::vector<DomNode*>& form_fields,
        std::vector<DomNode*>& images,
        const std::string& base_url
    );

    // 提取文档标题
    std::string extract_title(DomNode* root);

    // 从GumboNode提取所有文本
    std::string extract_text_from_gumbo(GumboNode* node);

    // 将GumboTag映射为ElementType
    ElementType map_gumbo_tag_to_element_type(int gumbo_tag);

    // URL解析
    std::string resolve_url(const std::string& url, const std::string& base_url);

    // HTML实体解码
    std::string decode_html_entities(const std::string& text);

    // HTML实体映射表
    static const std::map<std::string, std::string>& get_entity_map();
};