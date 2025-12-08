#include "text_renderer.h"
#include <sstream>
#include <algorithm>
#include <clocale>

class TextRenderer::Impl {
public:
    RenderConfig config;

    std::vector<std::string> wrap_text(const std::string& text, int width) {
        std::vector<std::string> lines;
        if (text.empty()) {
            return lines;
        }

        std::istringstream words_stream(text);
        std::string word;
        std::string current_line;

        while (words_stream >> word) {
            if (word.length() > static_cast<size_t>(width)) {
                if (!current_line.empty()) {
                    lines.push_back(current_line);
                    current_line.clear();
                }
                for (size_t i = 0; i < word.length(); i += width) {
                    lines.push_back(word.substr(i, width));
                }
                continue;
            }

            if (current_line.empty()) {
                current_line = word;
            } else if (current_line.length() + 1 + word.length() <= static_cast<size_t>(width)) {
                current_line += " " + word;
            } else {
                lines.push_back(current_line);
                current_line = word;
            }
        }

        if (!current_line.empty()) {
            lines.push_back(current_line);
        }

        if (lines.empty()) {
            lines.push_back("");
        }

        return lines;
    }

    std::string add_indent(const std::string& text, int indent) {
        return std::string(indent, ' ') + text;
    }
};

TextRenderer::TextRenderer() : pImpl(std::make_unique<Impl>()) {
    pImpl->config = RenderConfig();
}

TextRenderer::~TextRenderer() = default;

std::vector<RenderedLine> TextRenderer::render(const ParsedDocument& doc, int screen_width) {
    std::vector<RenderedLine> lines;

    int content_width = std::min(pImpl->config.max_width, screen_width - 4);
    if (content_width < 40) {
        content_width = screen_width - 4;
    }

    int margin = 0;
    if (pImpl->config.center_content && content_width < screen_width) {
        margin = (screen_width - content_width) / 2;
    }
    pImpl->config.margin_left = margin;

    if (!doc.title.empty()) {
        RenderedLine title_line;
        title_line.text = std::string(margin, ' ') + doc.title;
        title_line.color_pair = COLOR_HEADING1;
        title_line.is_bold = true;
        title_line.is_link = false;
        title_line.link_index = -1;
        lines.push_back(title_line);

        RenderedLine underline;
        underline.text = std::string(margin, ' ') + std::string(std::min((int)doc.title.length(), content_width), '=');
        underline.color_pair = COLOR_HEADING1;
        underline.is_bold = false;
        underline.is_link = false;
        underline.link_index = -1;
        lines.push_back(underline);

        RenderedLine empty;
        empty.text = "";
        empty.color_pair = COLOR_NORMAL;
        empty.is_bold = false;
        empty.is_link = false;
        empty.link_index = -1;
        lines.push_back(empty);
    }

    if (!doc.url.empty()) {
        RenderedLine url_line;
        url_line.text = std::string(margin, ' ') + "URL: " + doc.url;
        url_line.color_pair = COLOR_URL_BAR;
        url_line.is_bold = false;
        url_line.is_link = false;
        url_line.link_index = -1;
        lines.push_back(url_line);

        RenderedLine empty;
        empty.text = "";
        empty.color_pair = COLOR_NORMAL;
        empty.is_bold = false;
        empty.is_link = false;
        empty.link_index = -1;
        lines.push_back(empty);
    }

    for (const auto& elem : doc.elements) {
        int color = COLOR_NORMAL;
        bool bold = false;
        std::string prefix = "";

        switch (elem.type) {
            case ElementType::HEADING1:
                color = COLOR_HEADING1;
                bold = true;
                prefix = "# ";
                break;
            case ElementType::HEADING2:
                color = COLOR_HEADING2;
                bold = true;
                prefix = "## ";
                break;
            case ElementType::HEADING3:
                color = COLOR_HEADING3;
                bold = true;
                prefix = "### ";
                break;
            case ElementType::PARAGRAPH:
                color = COLOR_NORMAL;
                bold = false;
                break;
            case ElementType::BLOCKQUOTE:
                color = COLOR_DIM;
                prefix = "> ";
                break;
            case ElementType::LIST_ITEM:
                prefix = "  • ";
                break;
            case ElementType::HORIZONTAL_RULE:
                {
                    RenderedLine hr;
                    std::string hrline(content_width, '-');
                    hr.text = std::string(margin, ' ') + hrline;
                    hr.color_pair = COLOR_DIM;
                    hr.is_bold = false;
                    hr.is_link = false;
                    hr.link_index = -1;
                    lines.push_back(hr);
                    continue;
                }
            default:
                break;
        }

        auto wrapped_lines = pImpl->wrap_text(elem.text, content_width - prefix.length());
        for (size_t i = 0; i < wrapped_lines.size(); ++i) {
            RenderedLine line;
            if (i == 0) {
                line.text = std::string(margin, ' ') + prefix + wrapped_lines[i];
            } else {
                line.text = std::string(margin + prefix.length(), ' ') + wrapped_lines[i];
            }
            line.color_pair = color;
            line.is_bold = bold;
            line.is_link = false;
            line.link_index = -1;
            lines.push_back(line);
        }

        if (elem.type == ElementType::PARAGRAPH ||
            elem.type == ElementType::HEADING1 ||
            elem.type == ElementType::HEADING2 ||
            elem.type == ElementType::HEADING3) {
            for (int i = 0; i < pImpl->config.paragraph_spacing; ++i) {
                RenderedLine empty;
                empty.text = "";
                empty.color_pair = COLOR_NORMAL;
                empty.is_bold = false;
                empty.is_link = false;
                empty.link_index = -1;
                lines.push_back(empty);
            }
        }
    }

    if (!doc.links.empty() && pImpl->config.show_link_indicators) {
        RenderedLine separator;
        std::string sepline(content_width, '-');
        separator.text = std::string(margin, ' ') + sepline;
        separator.color_pair = COLOR_DIM;
        separator.is_bold = false;
        separator.is_link = false;
        separator.link_index = -1;
        lines.push_back(separator);

        RenderedLine links_header;
        links_header.text = std::string(margin, ' ') + "Links:";
        links_header.color_pair = COLOR_HEADING3;
        links_header.is_bold = true;
        links_header.is_link = false;
        links_header.link_index = -1;
        lines.push_back(links_header);

        RenderedLine empty;
        empty.text = "";
        empty.color_pair = COLOR_NORMAL;
        empty.is_bold = false;
        empty.is_link = false;
        empty.link_index = -1;
        lines.push_back(empty);

        for (size_t i = 0; i < doc.links.size(); ++i) {
            const auto& link = doc.links[i];
            std::string link_text = "[" + std::to_string(i) + "] " + link.text;

            auto wrapped = pImpl->wrap_text(link_text, content_width - 4);
            for (size_t j = 0; j < wrapped.size(); ++j) {
                RenderedLine link_line;
                link_line.text = std::string(margin + 2, ' ') + wrapped[j];
                link_line.color_pair = COLOR_LINK;
                link_line.is_bold = false;
                link_line.is_link = true;
                link_line.link_index = i;
                lines.push_back(link_line);
            }

            auto url_wrapped = pImpl->wrap_text(link.url, content_width - 6);
            for (const auto& url_line_text : url_wrapped) {
                RenderedLine url_line;
                url_line.text = std::string(margin + 4, ' ') + "→ " + url_line_text;
                url_line.color_pair = COLOR_DIM;
                url_line.is_bold = false;
                url_line.is_link = false;
                url_line.link_index = -1;
                lines.push_back(url_line);
            }

            lines.push_back(empty);
        }
    }

    return lines;
}

void TextRenderer::set_config(const RenderConfig& config) {
    pImpl->config = config;
}

RenderConfig TextRenderer::get_config() const {
    return pImpl->config;
}

void init_color_scheme() {
    if (has_colors()) {
        start_color();
        use_default_colors();

        init_pair(COLOR_NORMAL, COLOR_WHITE, -1);
        init_pair(COLOR_HEADING1, COLOR_CYAN, -1);
        init_pair(COLOR_HEADING2, COLOR_BLUE, -1);
        init_pair(COLOR_HEADING3, COLOR_MAGENTA, -1);
        init_pair(COLOR_LINK, COLOR_YELLOW, -1);
        init_pair(COLOR_LINK_ACTIVE, COLOR_BLACK, COLOR_YELLOW);
        init_pair(COLOR_STATUS_BAR, COLOR_BLACK, COLOR_WHITE);
        init_pair(COLOR_URL_BAR, COLOR_GREEN, -1);
        init_pair(COLOR_SEARCH_HIGHLIGHT, COLOR_BLACK, COLOR_YELLOW);
        init_pair(COLOR_DIM, COLOR_BLACK, -1);
    }
}
