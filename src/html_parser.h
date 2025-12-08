#pragma once

#include <string>
#include <vector>
#include <memory>

enum class ElementType {
    TEXT,
    HEADING1,
    HEADING2,
    HEADING3,
    PARAGRAPH,
    LINK,
    LIST_ITEM,
    BLOCKQUOTE,
    CODE_BLOCK,
    HORIZONTAL_RULE,
    LINE_BREAK
};

struct Link {
    std::string text;
    std::string url;
    int position; // 在文档中的位置（用于TAB导航）
};

struct ContentElement {
    ElementType type;
    std::string text;
    std::string url; // 对于链接元素
    int level;       // 对于标题元素（1-6）
};

struct ParsedDocument {
    std::string title;
    std::string url;
    std::vector<ContentElement> elements;
    std::vector<Link> links;
};

class HtmlParser {
public:
    HtmlParser();
    ~HtmlParser();

    // 解析HTML并提取可读内容
    ParsedDocument parse(const std::string& html, const std::string& base_url = "");

    // 设置是否保留代码块
    void set_keep_code_blocks(bool keep);

    // 设置是否保留列表
    void set_keep_lists(bool keep);

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
