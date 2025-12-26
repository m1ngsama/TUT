#pragma once
#include <string>
#include <cstddef>

namespace tut {

class Unicode {
public:
    /**
     * 计算字符串的显示宽度（考虑CJK、emoji）
     * ASCII=1, 2-byte=1, 3-byte(CJK)=2, 4-byte(emoji)=2
     */
    static size_t display_width(const std::string& text);

    /**
     * 获取UTF-8字符的字节长度
     */
    static size_t char_byte_length(const std::string& text, size_t pos);

    /**
     * 获取字符串中UTF-8字符的数量
     */
    static size_t char_count(const std::string& text);

    /**
     * 截取字符串到指定显示宽度
     * 返回截取后的字符串，不会截断多字节字符
     */
    static std::string truncate_to_width(const std::string& text, size_t max_width);

    /**
     * 填充字符串到指定显示宽度
     */
    static std::string pad_to_width(const std::string& text, size_t target_width, char pad_char = ' ');
};

} // namespace tut
