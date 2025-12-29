/**
 * @file bookmark_panel.hpp
 * @brief 书签面板组件
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <vector>
#include <functional>
#include <memory>

namespace tut {

/**
 * @brief 书签项
 */
struct BookmarkItem {
    std::string id;
    std::string title;
    std::string url;
    std::string folder;
    int64_t created_at{0};
    int64_t last_visited{0};
    int visit_count{0};
};

/**
 * @brief 书签面板组件类
 */
class BookmarkPanel {
public:
    BookmarkPanel();
    ~BookmarkPanel();

    /**
     * @brief 设置书签列表
     */
    void setBookmarks(const std::vector<BookmarkItem>& bookmarks);

    /**
     * @brief 获取所有书签
     */
    std::vector<BookmarkItem> getBookmarks() const;

    /**
     * @brief 添加书签
     */
    void addBookmark(const BookmarkItem& bookmark);

    /**
     * @brief 删除书签
     */
    void removeBookmark(const std::string& id);

    /**
     * @brief 选择下一个书签
     */
    void selectNext();

    /**
     * @brief 选择上一个书签
     */
    void selectPrevious();

    /**
     * @brief 获取选中的书签索引
     */
    int getSelectedIndex() const;

    /**
     * @brief 注册书签选择回调
     */
    void onSelect(std::function<void(const BookmarkItem&)> callback);

    /**
     * @brief 搜索书签
     */
    std::vector<BookmarkItem> search(const std::string& query) const;

    /**
     * @brief 显示/隐藏面板
     */
    void setVisible(bool visible);

    /**
     * @brief 是否可见
     */
    bool isVisible() const;

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
