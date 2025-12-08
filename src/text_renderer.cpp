#include "text_renderer.h"
#include <sstream>
#include <algorithm>
#include <clocale>

class TextRenderer::Impl {
public:
    RenderConfig config;

    struct LinkPosition {
        int link_index;
        size_t start;
        size_t end;
    };

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

    // Wrap text with links, tracking link positions and adding link numbers
    std::vector<std::pair<std::string, std::vector<LinkPosition>>>
    wrap_text_with_links(const std::string& original_text, int width,
                        const std::vector<InlineLink>& inline_links) {
        std::vector<std::pair<std::string, std::vector<LinkPosition>>> result;

        // If no links, use simple wrapping
        if (inline_links.empty()) {
            auto wrapped = wrap_text(original_text, width);
            for (const auto& line : wrapped) {
                result.push_back({line, {}});
            }
            return result;
        }

        // Build modified text with link numbers inserted
        std::string text;
        std::vector<InlineLink> modified_links;
        size_t text_pos = 0;

        for (const auto& link : inline_links) {
            // Add text before link
            if (link.start_pos > text_pos) {
                text += original_text.substr(text_pos, link.start_pos - text_pos);
            }

            // Add link text with number indicator
            size_t link_start_in_modified = text.length();
            std::string link_text = original_text.substr(link.start_pos, link.end_pos - link.start_pos);
            std::string link_indicator = "[" + std::to_string(link.link_index) + "]";
            text += link_text + link_indicator;

            // Store modified link position (including the indicator)
            InlineLink mod_link = link;
            mod_link.start_pos = link_start_in_modified;
            mod_link.end_pos = text.length();
            modified_links.push_back(mod_link);

            text_pos = link.end_pos;
        }

        // Add remaining text after last link
        if (text_pos < original_text.length()) {
            text += original_text.substr(text_pos);
        }

        // Split text into words
        std::vector<std::string> words;
        std::vector<size_t> word_positions;

        size_t pos = 0;
        while (pos < text.length()) {
            // Skip whitespace
            while (pos < text.length() && std::isspace(text[pos])) {
                pos++;
            }
            if (pos >= text.length()) break;

            // Extract word
            size_t word_start = pos;
            while (pos < text.length() && !std::isspace(text[pos])) {
                pos++;
            }

            words.push_back(text.substr(word_start, pos - word_start));
            word_positions.push_back(word_start);
        }

        // Build lines
        std::string current_line;
        std::vector<LinkPosition> current_links;

        for (size_t i = 0; i < words.size(); ++i) {
            const auto& word = words[i];
            size_t word_pos = word_positions[i];

            bool can_fit = current_line.empty()
                ? word.length() <= static_cast<size_t>(width)
                : current_line.length() + 1 + word.length() <= static_cast<size_t>(width);

            if (!can_fit && !current_line.empty()) {
                // Save current line
                result.push_back({current_line, current_links});
                current_line.clear();
                current_links.clear();
            }

            // Add word to current line
            if (!current_line.empty()) {
                current_line += " ";
            }
            size_t word_start_in_line = current_line.length();
            current_line += word;

            // Check if this word overlaps with any links
            for (const auto& link : modified_links) {
                size_t word_end = word_pos + word.length();

                // Check for overlap
                if (word_pos < link.end_pos && word_end > link.start_pos) {
                    // Calculate link position in current line
                    size_t link_start_in_line = word_start_in_line;
                    if (link.start_pos > word_pos) {
                        link_start_in_line += (link.start_pos - word_pos);
                    }

                    size_t link_end_in_line = word_start_in_line + word.length();
                    if (link.end_pos < word_end) {
                        link_end_in_line -= (word_end - link.end_pos);
                    }

                    // Check if link already added
                    bool already_added = false;
                    for (auto& existing : current_links) {
                        if (existing.link_index == link.link_index) {
                            // Extend existing link range
                            existing.end = link_end_in_line;
                            already_added = true;
                            break;
                        }
                    }

                    if (!already_added) {
                        LinkPosition lp;
                        lp.link_index = link.link_index;
                        lp.start = link_start_in_line;
                        lp.end = link_end_in_line;
                        current_links.push_back(lp);
                    }
                }
            }
        }

        // Add last line
        if (!current_line.empty()) {
            result.push_back({current_line, current_links});
        }

        if (result.empty()) {
            result.push_back({"", {}});
        }

        return result;
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

        auto wrapped_with_links = pImpl->wrap_text_with_links(elem.text,
                                                              content_width - prefix.length(),
                                                              elem.inline_links);

        for (size_t i = 0; i < wrapped_with_links.size(); ++i) {
            const auto& [line_text, link_positions] = wrapped_with_links[i];
            RenderedLine line;

            if (i == 0) {
                line.text = std::string(margin, ' ') + prefix + line_text;
            } else {
                line.text = std::string(margin + prefix.length(), ' ') + line_text;
            }

            line.color_pair = color;
            line.is_bold = bold;

            // Store link information
            if (!link_positions.empty()) {
                line.is_link = true;
                line.link_index = link_positions[0].link_index;  // Primary link for Tab navigation

                // Adjust link positions for margin and prefix
                size_t offset = (i == 0) ? (margin + prefix.length()) : (margin + prefix.length());
                for (const auto& lp : link_positions) {
                    line.link_ranges.push_back({lp.start + offset, lp.end + offset});
                }
            } else {
                line.is_link = false;
                line.link_index = -1;
            }

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

    // Don't show separate links section if inline links are displayed
    if (!doc.links.empty() && !pImpl->config.show_link_indicators) {
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
