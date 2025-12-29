/**
 * @file style_parser.cpp
 * @brief 样式解析实现
 */

#include "renderer/style_parser.hpp"
#include <sstream>
#include <algorithm>
#include <cmath>
#include <iomanip>

namespace tut {

const std::map<std::string, Color> StyleParser::named_colors_ = {
    {"black", {0, 0, 0}},
    {"white", {255, 255, 255}},
    {"red", {255, 0, 0}},
    {"green", {0, 128, 0}},
    {"blue", {0, 0, 255}},
    {"yellow", {255, 255, 0}},
    {"cyan", {0, 255, 255}},
    {"magenta", {255, 0, 255}},
    {"gray", {128, 128, 128}},
    {"grey", {128, 128, 128}},
    {"silver", {192, 192, 192}},
    {"maroon", {128, 0, 0}},
    {"olive", {128, 128, 0}},
    {"navy", {0, 0, 128}},
    {"purple", {128, 0, 128}},
    {"teal", {0, 128, 128}},
    {"orange", {255, 165, 0}},
    {"pink", {255, 192, 203}},
};

std::optional<Color> Color::fromHex(const std::string& hex) {
    std::string h = hex;
    if (!h.empty() && h[0] == '#') {
        h = h.substr(1);
    }

    if (h.length() == 3) {
        // 短格式 #RGB -> #RRGGBB
        h = std::string(2, h[0]) + std::string(2, h[1]) + std::string(2, h[2]);
    }

    if (h.length() != 6 && h.length() != 8) {
        return std::nullopt;
    }

    try {
        Color color;
        color.r = static_cast<uint8_t>(std::stoi(h.substr(0, 2), nullptr, 16));
        color.g = static_cast<uint8_t>(std::stoi(h.substr(2, 2), nullptr, 16));
        color.b = static_cast<uint8_t>(std::stoi(h.substr(4, 2), nullptr, 16));
        if (h.length() == 8) {
            color.a = static_cast<uint8_t>(std::stoi(h.substr(6, 2), nullptr, 16));
        }
        return color;
    } catch (...) {
        return std::nullopt;
    }
}

std::string Color::toHex() const {
    std::ostringstream oss;
    oss << "#" << std::hex << std::uppercase;
    oss << std::setw(2) << std::setfill('0') << static_cast<int>(r);
    oss << std::setw(2) << std::setfill('0') << static_cast<int>(g);
    oss << std::setw(2) << std::setfill('0') << static_cast<int>(b);
    return oss.str();
}

int Color::toAnsi256() const {
    // 转换为 ANSI 256 色
    if (r == g && g == b) {
        // 灰度
        if (r < 8) return 16;
        if (r > 248) return 231;
        return static_cast<int>(std::round((r - 8.0) / 247.0 * 24)) + 232;
    }

    // RGB 色
    int ri = static_cast<int>(std::round(r / 255.0 * 5));
    int gi = static_cast<int>(std::round(g / 255.0 * 5));
    int bi = static_cast<int>(std::round(b / 255.0 * 5));
    return 16 + 36 * ri + 6 * gi + bi;
}

std::string Color::toAnsiEscape(bool foreground) const {
    std::ostringstream oss;
    oss << "\033[" << (foreground ? "38" : "48") << ";2;"
        << static_cast<int>(r) << ";"
        << static_cast<int>(g) << ";"
        << static_cast<int>(b) << "m";
    return oss.str();
}

std::string TextStyle::toAnsiEscape() const {
    std::ostringstream oss;

    if (bold) oss << "\033[1m";
    if (italic) oss << "\033[3m";
    if (underline) oss << "\033[4m";
    if (strikethrough) oss << "\033[9m";

    if (foreground) {
        oss << foreground->toAnsiEscape(true);
    }
    if (background) {
        oss << background->toAnsiEscape(false);
    }

    return oss.str();
}

std::string TextStyle::resetAnsi() {
    return "\033[0m";
}

TextStyle StyleParser::parseInlineStyle(const std::string& style) {
    TextStyle result;

    // 简单的 CSS 解析
    std::istringstream iss(style);
    std::string declaration;

    while (std::getline(iss, declaration, ';')) {
        size_t colon = declaration.find(':');
        if (colon == std::string::npos) continue;

        std::string property = declaration.substr(0, colon);
        std::string value = declaration.substr(colon + 1);

        // 去除空白
        property.erase(0, property.find_first_not_of(" \t"));
        property.erase(property.find_last_not_of(" \t") + 1);
        value.erase(0, value.find_first_not_of(" \t"));
        value.erase(value.find_last_not_of(" \t") + 1);

        // 转换为小写
        std::transform(property.begin(), property.end(), property.begin(), ::tolower);
        std::transform(value.begin(), value.end(), value.begin(), ::tolower);

        if (property == "color") {
            result.foreground = parseColor(value);
        } else if (property == "background-color" || property == "background") {
            result.background = parseColor(value);
        } else if (property == "font-weight") {
            result.bold = (value == "bold" || value == "700" || value == "800" || value == "900");
        } else if (property == "font-style") {
            result.italic = (value == "italic" || value == "oblique");
        } else if (property == "text-decoration") {
            result.underline = (value.find("underline") != std::string::npos);
            result.strikethrough = (value.find("line-through") != std::string::npos);
        }
    }

    return result;
}

std::optional<Color> StyleParser::parseColor(const std::string& value) {
    if (value.empty()) return std::nullopt;

    // 十六进制颜色
    if (value[0] == '#') {
        return Color::fromHex(value);
    }

    // rgb() 格式
    if (value.substr(0, 4) == "rgb(") {
        size_t start = 4;
        size_t end = value.find(')');
        if (end == std::string::npos) return std::nullopt;

        std::string values = value.substr(start, end - start);
        std::istringstream iss(values);
        std::string token;
        std::vector<int> components;

        while (std::getline(iss, token, ',')) {
            try {
                components.push_back(std::stoi(token));
            } catch (...) {
                return std::nullopt;
            }
        }

        if (components.size() >= 3) {
            Color color;
            color.r = static_cast<uint8_t>(std::clamp(components[0], 0, 255));
            color.g = static_cast<uint8_t>(std::clamp(components[1], 0, 255));
            color.b = static_cast<uint8_t>(std::clamp(components[2], 0, 255));
            return color;
        }
    }

    // 命名颜色
    return getNamedColor(value);
}

std::optional<Color> StyleParser::getNamedColor(const std::string& name) {
    auto it = named_colors_.find(name);
    if (it != named_colors_.end()) {
        return it->second;
    }
    return std::nullopt;
}

}  // namespace tut
