#include "unicode.h"

namespace tut {

size_t Unicode::display_width(const std::string& text) {
    size_t width = 0;
    for (size_t i = 0; i < text.length(); ) {
        unsigned char c = text[i];

        if (c < 0x80) {
            // ASCII
            width += 1;
            i += 1;
        } else if ((c & 0xE0) == 0xC0) {
            // 2-byte UTF-8 (e.g., Latin extended)
            width += 1;
            i += 2;
        } else if ((c & 0xF0) == 0xE0) {
            // 3-byte UTF-8 (CJK characters)
            width += 2;
            i += 3;
        } else if ((c & 0xF8) == 0xF0) {
            // 4-byte UTF-8 (emoji, rare symbols)
            width += 2;
            i += 4;
        } else {
            // Invalid UTF-8, skip
            i += 1;
        }
    }
    return width;
}

size_t Unicode::char_byte_length(const std::string& text, size_t pos) {
    if (pos >= text.length()) return 0;

    unsigned char c = text[pos];
    if (c < 0x80) return 1;
    if ((c & 0xE0) == 0xC0) return 2;
    if ((c & 0xF0) == 0xE0) return 3;
    if ((c & 0xF8) == 0xF0) return 4;
    return 1; // Invalid, treat as single byte
}

size_t Unicode::char_count(const std::string& text) {
    size_t count = 0;
    for (size_t i = 0; i < text.length(); ) {
        i += char_byte_length(text, i);
        count++;
    }
    return count;
}

std::string Unicode::truncate_to_width(const std::string& text, size_t max_width) {
    std::string result;
    size_t current_width = 0;

    for (size_t i = 0; i < text.length(); ) {
        size_t byte_len = char_byte_length(text, i);
        unsigned char c = text[i];

        // Calculate width of this character
        size_t char_width = 1;
        if ((c & 0xF0) == 0xE0 || (c & 0xF8) == 0xF0) {
            char_width = 2; // CJK or emoji
        }

        if (current_width + char_width > max_width) {
            break;
        }

        result += text.substr(i, byte_len);
        current_width += char_width;
        i += byte_len;
    }

    return result;
}

std::string Unicode::pad_to_width(const std::string& text, size_t target_width, char pad_char) {
    size_t current_width = display_width(text);
    if (current_width >= target_width) return text;
    return text + std::string(target_width - current_width, pad_char);
}

} // namespace tut
