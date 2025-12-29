/**
 * @file main_window.hpp
 * @brief 主窗口模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <memory>
#include <functional>
#include <vector>

namespace tut {

/**
 * @brief 窗口事件类型
 */
enum class WindowEvent {
    None,
    Quit,
    Navigate,
    Back,
    Forward,
    Refresh,
    Search,
    AddBookmark,
    OpenBookmarks,
    OpenHistory,
    OpenSettings,
    OpenHelp,
};

/**
 * @brief 链接信息 (用于内容显示)
 */
struct DisplayLink {
    std::string text;
    std::string url;
    bool visited{false};
};

/**
 * @brief 书签信息 (用于显示)
 */
struct DisplayBookmark {
    std::string title;
    std::string url;
};

/**
 * @brief 主窗口类
 *
 * 负责整体 UI 布局和事件协调
 * 采用 btop 风格的四分区布局
 */
class MainWindow {
public:
    MainWindow();
    ~MainWindow();

    /**
     * @brief 初始化窗口
     */
    bool init();

    /**
     * @brief 运行主事件循环
     */
    int run();

    // ========== 状态设置 ==========

    void setStatusMessage(const std::string& message);
    void setUrl(const std::string& url);
    void setTitle(const std::string& title);
    void setContent(const std::string& content);
    void setLoading(bool loading);

    // ========== 内容管理 ==========

    /**
     * @brief 设置页面链接列表
     */
    void setLinks(const std::vector<DisplayLink>& links);

    /**
     * @brief 设置书签列表
     */
    void setBookmarks(const std::vector<DisplayBookmark>& bookmarks);

    /**
     * @brief 设置历史记录列表
     */
    void setHistory(const std::vector<DisplayBookmark>& history);

    /**
     * @brief 设置导航状态
     */
    void setCanGoBack(bool can);
    void setCanGoForward(bool can);

    // ========== 统计信息 ==========

    /**
     * @brief 设置加载统计
     */
    void setLoadStats(double elapsed_seconds, size_t bytes, int link_count);

    // ========== 回调注册 ==========

    void onNavigate(std::function<void(const std::string&)> callback);
    void onEvent(std::function<void(WindowEvent)> callback);
    void onLinkClick(std::function<void(int index)> callback);
    void onBookmarkClick(std::function<void(const std::string& url)> callback);

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
