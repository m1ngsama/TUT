#pragma once

#include <string>
#include <vector>
#include <ctime>

namespace tut {

/**
 * 书签条目
 */
struct Bookmark {
    std::string url;
    std::string title;
    std::time_t added_time;

    Bookmark() : added_time(0) {}
    Bookmark(const std::string& url, const std::string& title)
        : url(url), title(title), added_time(std::time(nullptr)) {}
};

/**
 * 书签管理器
 *
 * 书签存储在 ~/.config/tut/bookmarks.json
 */
class BookmarkManager {
public:
    BookmarkManager();
    ~BookmarkManager();

    /**
     * 加载书签（从默认路径）
     */
    bool load();

    /**
     * 保存书签（到默认路径）
     */
    bool save() const;

    /**
     * 添加书签
     * @return true 如果添加成功，false 如果已存在
     */
    bool add(const std::string& url, const std::string& title);

    /**
     * 删除书签
     * @return true 如果删除成功
     */
    bool remove(const std::string& url);

    /**
     * 删除书签（按索引）
     */
    bool remove_at(size_t index);

    /**
     * 检查URL是否已收藏
     */
    bool contains(const std::string& url) const;

    /**
     * 获取书签列表
     */
    const std::vector<Bookmark>& get_all() const { return bookmarks_; }

    /**
     * 获取书签数量
     */
    size_t count() const { return bookmarks_.size(); }

    /**
     * 清空所有书签
     */
    void clear() { bookmarks_.clear(); }

    /**
     * 获取配置目录路径
     */
    static std::string get_config_dir();

    /**
     * 获取书签文件路径
     */
    static std::string get_bookmarks_path();

private:
    std::vector<Bookmark> bookmarks_;

    // 确保配置目录存在
    static bool ensure_config_dir();
};

} // namespace tut
