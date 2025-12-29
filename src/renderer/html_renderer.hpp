/**
 * @file html_renderer.hpp
 * @brief HTML 渲染器模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <vector>
#include <memory>

namespace tut {

struct LinkInfo;

/**
 * @brief 渲染选项
 */
struct RenderOptions {
    int width{80};             ///< 渲染宽度
    bool show_links{true};     ///< 显示链接
    bool show_images{false};   ///< 显示图片 (ASCII art)
    bool use_colors{true};     ///< 使用颜色
    int indent_size{2};        ///< 缩进大小
};

/**
 * @brief 渲染结果
 */
struct RenderResult {
    std::string text;                ///< 渲染后的文本
    std::vector<LinkInfo> links;     ///< 提取的链接
    std::string title;               ///< 页面标题
    std::string description;         ///< 页面描述
};

/**
 * @brief HTML 渲染器类
 *
 * 将 HTML 文档渲染为终端可显示的文本
 */
class HtmlRenderer {
public:
    /**
     * @brief 构造函数
     */
    HtmlRenderer();

    /**
     * @brief 析构函数
     */
    ~HtmlRenderer();

    /**
     * @brief 渲染 HTML 文档
     * @param html HTML 字符串
     * @param options 渲染选项
     * @return 渲染结果
     */
    RenderResult render(const std::string& html,
                        const RenderOptions& options = RenderOptions{});

    /**
     * @brief 提取页面标题
     * @param html HTML 字符串
     * @return 标题
     */
    std::string extractTitle(const std::string& html);

    /**
     * @brief 提取所有链接
     * @param html HTML 字符串
     * @param base_url 基础 URL (用于解析相对链接)
     * @return 链接列表
     */
    std::vector<LinkInfo> extractLinks(const std::string& html,
                                        const std::string& base_url = "");

    /**
     * @brief 设置渲染选项
     */
    void setOptions(const RenderOptions& options);

    /**
     * @brief 获取渲染选项
     */
    const RenderOptions& getOptions() const;

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
