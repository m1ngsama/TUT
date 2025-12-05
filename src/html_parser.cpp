#include "html_parser.h"
#include <regex>
#include <algorithm>
#include <cctype>
#include <sstream>

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

    // 解析标题
    for (int level = 1; level <= 6; ++level) {
        std::string tag = "h" + std::to_string(level);
        auto headings = pImpl->extract_all_tags(main_content, tag);
        for (const auto& heading : headings) {
            ContentElement elem;
            elem.type = (level == 1) ? ElementType::HEADING1 :
                       (level == 2) ? ElementType::HEADING2 : ElementType::HEADING3;
            elem.text = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(heading)));
            elem.level = level;
            if (!elem.text.empty()) {
                doc.elements.push_back(elem);
            }
        }
    }

    // 解析列表项
    if (pImpl->keep_lists) {
        auto list_items = pImpl->extract_all_tags(main_content, "li");
        for (const auto& item : list_items) {
            std::string text = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(item)));
            if (!text.empty() && text.length() > 1) {
                ContentElement elem;
                elem.type = ElementType::LIST_ITEM;
                elem.text = text;
                doc.elements.push_back(elem);
            }
        }
    }

    // 解析段落
    auto paragraphs = pImpl->extract_all_tags(main_content, "p");
    for (const auto& para : paragraphs) {
        std::string text = pImpl->decode_html_entities(pImpl->trim(pImpl->remove_tags(para)));
        if (!text.empty() && text.length() > 1) {
            ContentElement elem;
            elem.type = ElementType::PARAGRAPH;
            elem.text = text;
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
