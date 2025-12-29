/**
 * @file browser_engine.hpp
 * @brief 浏览器引擎核心模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <memory>
#include <optional>
#include <vector>

namespace tut {

/**
 * @brief 链接信息结构体
 */
struct LinkInfo {
    std::string url;    ///< 链接 URL
    std::string text;   ///< 链接文本
    int line{0};        ///< 所在行号
};

/**
 * @brief 浏览器引擎类
 *
 * 负责协调 HTTP 请求、HTML 解析和渲染
 */
class BrowserEngine {
public:
    /**
     * @brief 默认构造函数
     */
    BrowserEngine();

    /**
     * @brief 析构函数
     */
    ~BrowserEngine();

    /**
     * @brief 加载指定 URL
     * @param url 要加载的 URL
     * @return 加载成功返回 true
     */
    bool loadUrl(const std::string& url);

    /**
     * @brief 直接加载 HTML 内容
     * @param html HTML 字符串
     * @return 加载成功返回 true
     */
    bool loadHtml(const std::string& html);

    /**
     * @brief 获取页面标题
     * @return 页面标题，如果没有则返回空字符串
     */
    std::string getTitle() const;

    /**
     * @brief 获取当前 URL
     * @return 当前 URL
     */
    std::string getCurrentUrl() const;

    /**
     * @brief 提取页面中的链接
     * @return 链接列表
     */
    std::vector<LinkInfo> extractLinks() const;

    /**
     * @brief 获取渲染后的文本内容
     * @return 渲染后的文本
     */
    std::string getRenderedContent() const;

    /**
     * @brief 后退到上一页
     * @return 后退成功返回 true
     */
    bool goBack();

    /**
     * @brief 前进到下一页
     * @return 前进成功返回 true
     */
    bool goForward();

    /**
     * @brief 刷新当前页面
     * @return 刷新成功返回 true
     */
    bool refresh();

    /**
     * @brief 检查是否可以后退
     * @return 可以后退返回 true
     */
    bool canGoBack() const;

    /**
     * @brief 检查是否可以前进
     * @return 可以前进返回 true
     */
    bool canGoForward() const;

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
