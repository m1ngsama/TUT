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
    int position;
};

struct InlineLink {
    std::string text;
    std::string url;
    size_t start_pos;  // Position in the text where link starts
    size_t end_pos;    // Position in the text where link ends
    int link_index;    // Index in the document's links array
};

struct ContentElement {
    ElementType type;
    std::string text;
    std::string url;
    int level;
    std::vector<InlineLink> inline_links;  // Links within this element's text
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

    ParsedDocument parse(const std::string& html, const std::string& base_url = "");
    void set_keep_code_blocks(bool keep);
    void set_keep_lists(bool keep);

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
