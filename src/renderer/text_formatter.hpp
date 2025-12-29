/**
 * @file text_formatter.hpp
 * @brief 文本格式化模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <vector>

namespace tut {

/**
 * @brief 文本格式化器类
 *
 * 负责文本的格式化、换行、对齐等操作
 */
class TextFormatter {
public:
    /**
     * @brief 自动换行
     * @param text 输入文本
     * @param width 每行最大宽度
     * @return 换行后的文本行
     */
    static std::vector<std::string> wrapText(const std::string& text, int width);

    /**
     * @brief 左对齐
     * @param text 输入文本
     * @param width 目标宽度
     * @return 左对齐后的文本
     */
    static std::string alignLeft(const std::string& text, int width);

    /**
     * @brief 右对齐
     * @param text 输入文本
     * @param width 目标宽度
     * @return 右对齐后的文本
     */
    static std::string alignRight(const std::string& text, int width);

    /**
     * @brief 居中对齐
     * @param text 输入文本
     * @param width 目标宽度
     * @return 居中对齐后的文本
     */
    static std::string alignCenter(const std::string& text, int width);

    /**
     * @brief 截断文本
     * @param text 输入文本
     * @param max_length 最大长度
     * @param suffix 截断后缀 (默认 "...")
     * @return 截断后的文本
     */
    static std::string truncate(const std::string& text, size_t max_length,
                                const std::string& suffix = "...");

    /**
     * @brief 去除首尾空白
     * @param text 输入文本
     * @return 处理后的文本
     */
    static std::string trim(const std::string& text);

    /**
     * @brief 计算显示宽度 (考虑 Unicode 字符)
     * @param text 输入文本
     * @return 显示宽度
     */
    static int displayWidth(const std::string& text);

    /**
     * @brief 将制表符转换为空格
     * @param text 输入文本
     * @param tab_size 制表符大小
     * @return 转换后的文本
     */
    static std::string expandTabs(const std::string& text, int tab_size = 4);

    /**
     * @brief 规范化空白字符
     * @param text 输入文本
     * @return 规范化后的文本
     */
    static std::string normalizeWhitespace(const std::string& text);
};

}  // namespace tut
