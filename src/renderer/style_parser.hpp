/**
 * @file style_parser.hpp
 * @brief 样式解析模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <map>
#include <optional>

namespace tut {

/**
 * @brief 颜色结构体
 */
struct Color {
    uint8_t r{0};
    uint8_t g{0};
    uint8_t b{0};
    uint8_t a{255};

    bool operator==(const Color& other) const {
        return r == other.r && g == other.g && b == other.b && a == other.a;
    }

    /**
     * @brief 从十六进制字符串解析
     * @param hex 十六进制颜色 (如 "#FF0000" 或 "FF0000")
     */
    static std::optional<Color> fromHex(const std::string& hex);

    /**
     * @brief 转换为十六进制字符串
     */
    std::string toHex() const;

    /**
     * @brief 转换为 ANSI 256 色
     */
    int toAnsi256() const;

    /**
     * @brief 转换为 ANSI 转义序列
     * @param foreground 是否是前景色
     */
    std::string toAnsiEscape(bool foreground = true) const;
};

/**
 * @brief 文本样式
 */
struct TextStyle {
    std::optional<Color> foreground;
    std::optional<Color> background;
    bool bold{false};
    bool italic{false};
    bool underline{false};
    bool strikethrough{false};

    /**
     * @brief 转换为 ANSI 转义序列
     */
    std::string toAnsiEscape() const;

    /**
     * @brief 重置 ANSI 格式
     */
    static std::string resetAnsi();
};

/**
 * @brief 样式解析器类
 *
 * 解析基础的 CSS 样式
 */
class StyleParser {
public:
    /**
     * @brief 解析内联样式
     * @param style CSS 样式字符串
     * @return 解析后的样式
     */
    static TextStyle parseInlineStyle(const std::string& style);

    /**
     * @brief 解析颜色值
     * @param value CSS 颜色值
     * @return 解析后的颜色
     */
    static std::optional<Color> parseColor(const std::string& value);

    /**
     * @brief 获取命名颜色
     * @param name 颜色名称
     * @return 颜色值
     */
    static std::optional<Color> getNamedColor(const std::string& name);

private:
    static const std::map<std::string, Color> named_colors_;
};

}  // namespace tut
