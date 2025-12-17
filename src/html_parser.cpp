#include "html_parser.h"
#include <regex>
#include <algorithm>
#include <cctype>
#include <sstream>
#include <functional>

class HtmlParser::Impl {
public:
    bool keep_code_blocks = true;
    bool keep_lists = true;

    // 简单的HTML标签清理
    std::string remove_tags(const std::string& html) {
        std::string result;
        bool in_tag = false;
        for (char c : html) {
            if (c == '<') {
                in_tag = true;
            } else if (c == '>') {
                in_tag = false;
            } else if (!in_tag) {
                result += c;
            }
        }
        return result;
    }

    // 解码HTML实体
    std::string decode_html_entities(const std::string& text) {
        std::string result = text;

        // 常见HTML实体
        const std::vector<std::pair<std::string, std::string>> entities = {
            {"&nbsp;", " "},
            {"&amp;", "&"},
            {"&lt;", "<"},
            {"&gt;", ">"},
            {"&quot;", "\""},
            {"&apos;", "'"},
            {"&#39;", "'"},
            {"&mdash;", "\u2014"},
            {"&ndash;", "\u2013"},
            {"&hellip;", "..."},
            {"&ldquo;", "\u201C"},
            {"&rdquo;", "\u201D"},
            {"&lsquo;", "\u2018"},
            {"&rsquo;", "\u2019"}
        };

        for (const auto& [entity, replacement] : entities) {
            size_t pos = 0;
            while ((pos = result.find(entity, pos)) != std::string::npos) {
                result.replace(pos, entity.length(), replacement);
                pos += replacement.length();
            }
        }

        return result;
    }

    // 提取标签内容
    std::string extract_tag_content(const std::string& html, const std::string& tag) {
        std::regex tag_regex("<" + tag + "[^>]*>([\\s\\S]*?)</" + tag + ">",
                           std::regex::icase);
        std::smatch match;
        if (std::regex_search(html, match, tag_regex)) {
            return match[1].str();
        }
        return "";
    }

    // 提取所有匹配的标签
    std::vector<std::string> extract_all_tags(const std::string& html, const std::string& tag) {
        std::vector<std::string> results;
        std::regex tag_regex("<" + tag + "[^>]*>([\\s\\S]*?)</" + tag + ">",
                           std::regex::icase);

        auto begin = std::sregex_iterator(html.begin(), html.end(), tag_regex);
        auto end = std::sregex_iterator();

        for (std::sregex_iterator i = begin; i != end; ++i) {
            std::smatch match = *i;
            results.push_back(match[1].str());
        }

        return results;
    }

    // 提取链接
    std::vector<Link> extract_links(const std::string& html, const std::string& base_url) {
        std::vector<Link> links;
        std::regex link_regex(R"(<a\s+[^>]*href\s*=\s*["']([^"']*)["'][^>]*>([\s\S]*?)</a>)",
                            std::regex::icase);

        auto begin = std::sregex_iterator(html.begin(), html.end(), link_regex);
        auto end = std::sregex_iterator();

        int position = 0;
        for (std::sregex_iterator i = begin; i != end; ++i) {
            std::smatch match = *i;
            Link link;
            link.url = match[1].str();
            link.text = decode_html_entities(remove_tags(match[2].str()));
            link.position = position++;

            // 处理相对URL
            if (!link.url.empty() && link.url[0] != '#') {
                // 如果是相对路径
                if (link.url.find("://") == std::string::npos) {
                    // 提取base_url的协议和域名
                    std::regex base_regex(R"((https?://[^/]+)(/.*)?)", std::regex::icase);
                    std::smatch base_match;
                    if (std::regex_match(base_url, base_match, base_regex)) {
                        std::string base_domain = base_match[1].str();
                        std::string base_path = base_match[2].str();

                        if (link.url[0] == '/') {
                            // 绝对路径（从根目录开始）
                            link.url = base_domain + link.url;
                        } else {
                            // 相对路径
                            // 获取当前页面的目录
                            size_t last_slash = base_path.rfind('/');
                            std::string current_dir = (last_slash != std::string::npos)
                                ? base_path.substr(0, last_slash + 1)
                                : "/";
                            link.url = base_domain + current_dir + link.url;
                        }
                    }
                }

                // 过滤空链接文本
                if (!link.text.empty()) {
                    links.push_back(link);
                }
            }
        }

        return links;
    }

    // 从HTML中提取文本，同时保留内联链接位置信息
    std::string extract_text_with_links(const std::string& html,
                                        std::vector<Link>& all_links,
                                        std::vector<InlineLink>& inline_links) {
        std::string result;
        std::regex link_regex(R"(<a\s+[^>]*href\s*=\s*["']([^"']*)["'][^>]*>([\s\S]*?)</a>)",
                            std::regex::icase);

        size_t last_pos = 0;
        auto begin = std::sregex_iterator(html.begin(), html.end(), link_regex);
        auto end = std::sregex_iterator();

        // 处理所有链接
        for (std::sregex_iterator i = begin; i != end; ++i) {
            std::smatch match = *i;

            // 添加链接前的文本
            std::string before_link = html.substr(last_pos, match.position() - last_pos);
            std::string before_text = decode_html_entities(remove_tags(before_link));
            result += before_text;

            // 提取链接信息
            std::string link_url = match[1].str();
            std::string link_text = decode_html_entities(remove_tags(match[2].str()));

            // 跳过空链接或锚点链接
            if (link_url.empty() || link_url[0] == '#' || link_text.empty()) {
                result += link_text;
                last_pos = match.position() + match.length();
                continue;
            }

            // 找到这个链接在全局链接列表中的索引
            int link_index = -1;
            for (size_t j = 0; j < all_links.size(); ++j) {
                if (all_links[j].url == link_url && all_links[j].text == link_text) {
                    link_index = j;
                    break;
                }
            }

            if (link_index != -1) {
                // 记录内联链接位置
                InlineLink inline_link;
                inline_link.text = link_text;
                inline_link.url = link_url;
                inline_link.start_pos = result.length();
                inline_link.end_pos = result.length() + link_text.length();
                inline_link.link_index = link_index;
                inline_links.push_back(inline_link);
            }

            // 添加链接文本
            result += link_text;
            last_pos = match.position() + match.length();
        }

        // 添加最后一段文本
        std::string remaining = html.substr(last_pos);
        result += decode_html_entities(remove_tags(remaining));

        return trim(result);
    }

    // 清理空白字符
    std::string trim(const std::string& str) {
        auto start = str.begin();
        while (start != str.end() && std::isspace(*start)) {
            ++start;
        }

        auto end = str.end();
        do {
            --end;
        } while (std::distance(start, end) > 0 && std::isspace(*end));

        return std::string(start, end + 1);
    }

    // 移除脚本和样式
    std::string remove_scripts_and_styles(const std::string& html) {
        std::string result = html;

        // 移除script标签
        result = std::regex_replace(result,
            std::regex("<script[^>]*>[\\s\\S]*?</script>", std::regex::icase),
            "");

        // 移除style标签
        result = std::regex_replace(result,
            std::regex("<style[^>]*>[\\s\\S]*?</style>", std::regex::icase),
            "");

        return result;
    }

    // Extract images
    std::vector<Image> extract_images(const std::string& html) {
        std::vector<Image> images;
        std::regex img_regex(R"(<img[^>]*src\s*=\s*["']([^"']*)["'][^>]*>)", std::regex::icase);

        auto begin = std::sregex_iterator(html.begin(), html.end(), img_regex);
        auto end = std::sregex_iterator();

        for (std::sregex_iterator i = begin; i != end; ++i) {
            std::smatch match = *i;
            Image img;
            img.src = match[1].str();
            img.width = -1;
            img.height = -1;

            // Extract alt text
            std::string img_tag = match[0].str();
            std::regex alt_regex(R"(alt\s*=\s*["']([^"']*)["'])", std::regex::icase);
            std::smatch alt_match;
            if (std::regex_search(img_tag, alt_match, alt_regex)) {
                img.alt = decode_html_entities(alt_match[1].str());
            }

            // Extract width
            std::regex width_regex(R"(width\s*=\s*["']?(\d+)["']?)", std::regex::icase);
            std::smatch width_match;
            if (std::regex_search(img_tag, width_match, width_regex)) {
                try {
                    img.width = std::stoi(width_match[1].str());
                } catch (...) {}
            }

            // Extract height
            std::regex height_regex(R"(height\s*=\s*["']?(\d+)["']?)", std::regex::icase);
            std::smatch height_match;
            if (std::regex_search(img_tag, height_match, height_regex)) {
                try {
                    img.height = std::stoi(height_match[1].str());
                } catch (...) {}
            }

            images.push_back(img);
        }

        return images;
    }

    // Extract tables
    std::vector<Table> extract_tables(const std::string& html, std::vector<Link>& all_links) {
        std::vector<Table> tables;
        auto table_contents = extract_all_tags(html, "table");

        for (const auto& table_html : table_contents) {
            Table table;
            table.has_header = false;

            // Extract rows
            auto thead_html = extract_tag_content(table_html, "thead");
            auto tbody_html = extract_tag_content(table_html, "tbody");

            // If no thead/tbody, just get all rows
            std::vector<std::string> row_htmls;
            if (!thead_html.empty() || !tbody_html.empty()) {
                if (!thead_html.empty()) {
                    auto header_rows = extract_all_tags(thead_html, "tr");
                    row_htmls.insert(row_htmls.end(), header_rows.begin(), header_rows.end());
                    table.has_header = !header_rows.empty();
                }
                if (!tbody_html.empty()) {
                    auto body_rows = extract_all_tags(tbody_html, "tr");
                    row_htmls.insert(row_htmls.end(), body_rows.begin(), body_rows.end());
                }
            } else {
                row_htmls = extract_all_tags(table_html, "tr");
                // Check if first row has <th> tags
                if (!row_htmls.empty()) {
                    table.has_header = (row_htmls[0].find("<th") != std::string::npos);
                }
            }

            bool is_first_row = true;
            for (const auto& row_html : row_htmls) {
                TableRow row;

                // Extract cells (both th and td)
                auto th_cells = extract_all_tags(row_html, "th");
                auto td_cells = extract_all_tags(row_html, "td");

                // Process th cells (headers)
                for (const auto& cell_html : th_cells) {
                    TableCell cell;
                    std::vector<InlineLink> inline_links;
                    cell.text = extract_text_with_links(cell_html, all_links, inline_links);
                    cell.inline_links = inline_links;
                    cell.is_header = true;
                    cell.colspan = 1;
                    cell.rowspan = 1;
                    row.cells.push_back(cell);
                }

                // Process td cells (data)
                for (const auto& cell_html : td_cells) {
                    TableCell cell;
                    std::vector<InlineLink> inline_links;
                    cell.text = extract_text_with_links(cell_html, all_links, inline_links);
                    cell.inline_links = inline_links;
                    cell.is_header = is_first_row && table.has_header && th_cells.empty();
                    cell.colspan = 1;
                    cell.rowspan = 1;
                    row.cells.push_back(cell);
                }

                if (!row.cells.empty()) {
                    table.rows.push_back(row);
                }

                is_first_row = false;
            }

            if (!table.rows.empty()) {
                tables.push_back(table);
            }
        }

        return tables;
    }
};

HtmlParser::HtmlParser() : pImpl(std::make_unique<Impl>()) {}

HtmlParser::~HtmlParser() = default;

ParsedDocument HtmlParser::parse(const std::string& html, const std::string& base_url) {
    ParsedDocument doc;
    doc.url = base_url;

    // 清理HTML
    std::string clean_html = pImpl->remove_scripts_and_styles(html);

    // 提取标题
    std::string title_content = pImpl->extract_tag_content(clean_html, "title");
    doc.title = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(title_content)));

    if (doc.title.empty()) {
        std::string h1_content = pImpl->extract_tag_content(clean_html, "h1");
        doc.title = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(h1_content)));
    }

    // 提取主要内容区域（article, main, 或 body）
    std::string main_content = pImpl->extract_tag_content(clean_html, "article");
    if (main_content.empty()) {
        main_content = pImpl->extract_tag_content(clean_html, "main");
    }
    if (main_content.empty()) {
        main_content = pImpl->extract_tag_content(clean_html, "body");
    }
    if (main_content.empty()) {
        main_content = clean_html;
    }

    // 提取链接
    doc.links = pImpl->extract_links(main_content, base_url);

    // Extract and add images
    auto images = pImpl->extract_images(main_content);
    for (const auto& img : images) {
        ContentElement elem;
        elem.type = ElementType::IMAGE;
        elem.image_data = img;
        elem.level = 0;
        elem.list_number = 0;
        elem.nesting_level = 0;
        doc.elements.push_back(elem);
    }

    // Extract and add tables
    auto tables = pImpl->extract_tables(main_content, doc.links);
    for (const auto& tbl : tables) {
        ContentElement elem;
        elem.type = ElementType::TABLE;
        elem.table_data = tbl;
        elem.level = 0;
        elem.list_number = 0;
        elem.nesting_level = 0;
        doc.elements.push_back(elem);
    }

    // 解析标题
    for (int level = 1; level <= 6; ++level) {
        std::string tag = "h" + std::to_string(level);
        auto headings = pImpl->extract_all_tags(main_content, tag);
        for (const auto& heading : headings) {
            ContentElement elem;
            ElementType type;
            if (level == 1) type = ElementType::HEADING1;
            else if (level == 2) type = ElementType::HEADING2;
            else if (level == 3) type = ElementType::HEADING3;
            else if (level == 4) type = ElementType::HEADING4;
            else if (level == 5) type = ElementType::HEADING5;
            else type = ElementType::HEADING6;

            elem.type = type;
            elem.text = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(heading)));
            elem.level = level;
            elem.list_number = 0;
            elem.nesting_level = 0;
            if (!elem.text.empty()) {
                doc.elements.push_back(elem);
            }
        }
    }

    // 解析列表项 - with nesting support
    if (pImpl->keep_lists) {
        // Extract both <ul> and <ol> lists
        auto ul_lists = pImpl->extract_all_tags(main_content, "ul");
        auto ol_lists = pImpl->extract_all_tags(main_content, "ol");

        // Helper to parse a list recursively
        std::function<void(const std::string&, bool, int)> parse_list;
        parse_list = [&](const std::string& list_html, bool is_ordered, int nesting) {
            auto list_items = pImpl->extract_all_tags(list_html, "li");
            int item_number = 1;

            for (const auto& item_html : list_items) {
                // Check if this item contains nested lists
                bool has_nested_ul = item_html.find("<ul") != std::string::npos;
                bool has_nested_ol = item_html.find("<ol") != std::string::npos;

                // Extract text without nested lists
                std::string item_text = item_html;
                if (has_nested_ul || has_nested_ol) {
                    // Remove nested lists from text
                    item_text = std::regex_replace(item_text,
                        std::regex("<ul[^>]*>[\\s\\S]*?</ul>", std::regex::icase), "");
                    item_text = std::regex_replace(item_text,
                        std::regex("<ol[^>]*>[\\s\\S]*?</ol>", std::regex::icase), "");
                }

                std::string text = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(item_text)));
                if (!text.empty() && text.length() > 1) {
                    ContentElement elem;
                    elem.type = is_ordered ? ElementType::ORDERED_LIST_ITEM : ElementType::LIST_ITEM;
                    elem.text = text;
                    elem.level = 0;
                    elem.list_number = item_number++;
                    elem.nesting_level = nesting;
                    doc.elements.push_back(elem);
                }

                // Parse nested lists
                if (has_nested_ul) {
                    auto nested_uls = pImpl->extract_all_tags(item_html, "ul");
                    for (const auto& nested_ul : nested_uls) {
                        parse_list(nested_ul, false, nesting + 1);
                    }
                }
                if (has_nested_ol) {
                    auto nested_ols = pImpl->extract_all_tags(item_html, "ol");
                    for (const auto& nested_ol : nested_ols) {
                        parse_list(nested_ol, true, nesting + 1);
                    }
                }
            }
        };

        // Parse unordered lists
        for (const auto& ul : ul_lists) {
            parse_list(ul, false, 0);
        }

        // Parse ordered lists
        for (const auto& ol : ol_lists) {
            parse_list(ol, true, 0);
        }
    }

    // 解析段落 (保留内联链接)
    auto paragraphs = pImpl->extract_all_tags(main_content, "p");
    for (const auto& para : paragraphs) {
        ContentElement elem;
        elem.type = ElementType::PARAGRAPH;
        elem.text = pImpl->extract_text_with_links(para, doc.links, elem.inline_links);
        elem.level = 0;
        elem.list_number = 0;
        elem.nesting_level = 0;
        if (!elem.text.empty() && elem.text.length() > 1) {
            doc.elements.push_back(elem);
        }
    }

    // 如果内容很少，尝试提取div中的文本
    if (doc.elements.size() < 3) {
        auto divs = pImpl->extract_all_tags(main_content, "div");
        for (const auto& div : divs) {
            std::string text = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(div)));
            if (!text.empty() && text.length() > 20) {  // 忽略太短的div
                ContentElement elem;
                elem.type = ElementType::PARAGRAPH;
                elem.text = text;
                elem.level = 0;
                elem.list_number = 0;
                elem.nesting_level = 0;
                doc.elements.push_back(elem);
            }
        }
    }

    // 如果仍然没有内容，尝试提取整个文本
    if (doc.elements.empty()) {
        std::string all_text = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(main_content)));
        if (!all_text.empty()) {
            // 按换行符分割
            std::istringstream iss(all_text);
            std::string line;
            while (std::getline(iss, line)) {
                line = pImpl->trim(line);
                if (!line.empty() && line.length() > 1) {
                    ContentElement elem;
                    elem.type = ElementType::PARAGRAPH;
                    elem.text = line;
                    elem.level = 0;
                    elem.list_number = 0;
                    elem.nesting_level = 0;
                    doc.elements.push_back(elem);
                }
            }
        }
    }

    return doc;
}

void HtmlParser::set_keep_code_blocks(bool keep) {
    pImpl->keep_code_blocks = keep;
}

void HtmlParser::set_keep_lists(bool keep) {
    pImpl->keep_lists = keep;
}
