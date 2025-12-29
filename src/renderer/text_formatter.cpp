/**
 * @file text_formatter.cpp
 * @brief 文本格式化实现
 */

#include "renderer/text_formatter.hpp"
#include <algorithm>
#include <sstream>

namespace tut {

std::vector<std::string> TextFormatter::wrapText(const std::string& text, int width) {
    std::vector<std::string> lines;
    if (width <= 0 || text.empty()) {
        return lines;
    }

    std::istringstream iss(text);
    std::string word;
    std::string current_line;

    while (iss >> word) {
        if (current_line.empty()) {
            current_line = word;
        } else if (static_cast<int>(current_line.length() + 1 + word.length()) <= width) {
            current_line += " " + word;
        } else {
            lines.push_back(current_line);
            current_line = word;
        }
    }

    if (!current_line.empty()) {
        lines.push_back(current_line);
    }

    return lines;
}

std::string TextFormatter::alignLeft(const std::string& text, int width) {
    if (static_cast<int>(text.length()) >= width) {
        return text;
    }
    return text + std::string(width - text.length(), ' ');
}

std::string TextFormatter::alignRight(const std::string& text, int width) {
    if (static_cast<int>(text.length()) >= width) {
        return text;
    }
    return std::string(width - text.length(), ' ') + text;
}

std::string TextFormatter::alignCenter(const std::string& text, int width) {
    if (static_cast<int>(text.length()) >= width) {
        return text;
    }
    int padding = width - static_cast<int>(text.length());
    int left_padding = padding / 2;
    int right_padding = padding - left_padding;
    return std::string(left_padding, ' ') + text + std::string(right_padding, ' ');
}

std::string TextFormatter::truncate(const std::string& text, size_t max_length,
                                     const std::string& suffix) {
    if (text.length() <= max_length) {
        return text;
    }
    if (max_length <= suffix.length()) {
        return suffix.substr(0, max_length);
    }
    return text.substr(0, max_length - suffix.length()) + suffix;
}

std::string TextFormatter::trim(const std::string& text) {
    size_t start = text.find_first_not_of(" \t\n\r\f\v");
    if (start == std::string::npos) {
        return "";
    }
    size_t end = text.find_last_not_of(" \t\n\r\f\v");
    return text.substr(start, end - start + 1);
}

int TextFormatter::displayWidth(const std::string& text) {
    int width = 0;
    for (size_t i = 0; i < text.length(); ) {
        unsigned char c = text[i];
        if ((c & 0x80) == 0) {
            // ASCII
            width += 1;
            i += 1;
        } else if ((c & 0xE0) == 0xC0) {
            // 2-byte UTF-8
            width += 1;
            i += 2;
        } else if ((c & 0xF0) == 0xE0) {
            // 3-byte UTF-8 (CJK 字符通常是这种)
            width += 2;  // 假设是宽字符
            i += 3;
        } else if ((c & 0xF8) == 0xF0) {
            // 4-byte UTF-8
            width += 2;
            i += 4;
        } else {
            width += 1;
            i += 1;
        }
    }
    return width;
}

std::string TextFormatter::expandTabs(const std::string& text, int tab_size) {
    std::string result;
    int column = 0;

    for (char c : text) {
        if (c == '\t') {
            int spaces = tab_size - (column % tab_size);
            result.append(spaces, ' ');
            column += spaces;
        } else if (c == '\n') {
            result += c;
            column = 0;
        } else {
            result += c;
            column++;
        }
    }

    return result;
}

std::string TextFormatter::normalizeWhitespace(const std::string& text) {
    std::string result;
    bool last_was_space = false;

    for (char c : text) {
        if (std::isspace(static_cast<unsigned char>(c))) {
            if (!last_was_space) {
                result += ' ';
                last_was_space = true;
            }
        } else {
            result += c;
            last_was_space = false;
        }
    }

    return trim(result);
}

}  // namespace tut
