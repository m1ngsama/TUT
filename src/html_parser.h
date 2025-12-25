#pragma once

#include <string>
#include <vector>
#include <memory>

// Forward declaration
struct DocumentTree;

enum class ElementType {
    TEXT,
    HEADING1,
    HEADING2,
    HEADING3,
    HEADING4,
    HEADING5,
    HEADING6,
    PARAGRAPH,
    LINK,
    LIST_ITEM,
    ORDERED_LIST_ITEM,
    BLOCKQUOTE,
    CODE_BLOCK,
    HORIZONTAL_RULE,
    LINE_BREAK,
    TABLE,
    IMAGE,
    FORM,
    INPUT,
    TEXTAREA,
    SELECT,
    OPTION,
    BUTTON,
    SECTION_START,
    SECTION_END,
    NAV_START,
    NAV_END,
    HEADER_START,
    HEADER_END,
    ASIDE_START,
    ASIDE_END
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
    int field_index = -1; // Index in the document's form_fields array
};

struct TableCell {
    std::string text;
    std::vector<InlineLink> inline_links;
    bool is_header;
    int colspan;
    int rowspan;
};

struct TableRow {
    std::vector<TableCell> cells;
};

struct Table {
    std::vector<TableRow> rows;
    bool has_header;
};

struct Image {
    std::string src;
    std::string alt;
    int width;   // -1 if not specified
    int height;  // -1 if not specified
};

struct FormField {
    enum Type { TEXT, PASSWORD, CHECKBOX, RADIO, SUBMIT, BUTTON } type;
    std::string name;
    std::string value;
    std::string placeholder;
    bool checked;
};

struct Form {
    std::string action;
    std::string method;
    std::vector<FormField> fields;
};

struct ContentElement {
    ElementType type;
    std::string text;
    std::string url;
    int level;
    int list_number;  // For ordered lists
    int nesting_level;  // For nested lists
    std::vector<InlineLink> inline_links;  // Links within this element's text

    // Extended content types
    Table table_data;
    Image image_data;
    Form form_data;
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

    // 新接口：使用DOM树解析
    DocumentTree parse_tree(const std::string& html, const std::string& base_url = "");

    // 旧接口：保持向后兼容（已废弃，内部使用parse_tree）
    ParsedDocument parse(const std::string& html, const std::string& base_url = "");

    void set_keep_code_blocks(bool keep);
    void set_keep_lists(bool keep);

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
