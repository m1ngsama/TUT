/**
 * @file bookmark_manager.hpp
 * @brief Bookmark manager for persistent storage
 * @author m1ngsama
 * @date 2025-01-01
 */

#pragma once

#include <string>
#include <vector>
#include <memory>

namespace tut {

/**
 * @brief Bookmark entry
 */
struct Bookmark {
    std::string title;
    std::string url;
    int64_t timestamp{0};  // Unix timestamp

    Bookmark() = default;
    Bookmark(const std::string& t, const std::string& u, int64_t ts = 0)
        : title(t), url(u), timestamp(ts) {}
};

/**
 * @brief Bookmark manager with JSON persistence
 *
 * Manages bookmarks with automatic persistence to
 * ~/.config/tut/bookmarks.json
 */
class BookmarkManager {
public:
    BookmarkManager();
    ~BookmarkManager();

    /**
     * @brief Add a bookmark
     * @return true if added, false if already exists
     */
    bool add(const std::string& title, const std::string& url);

    /**
     * @brief Remove a bookmark by URL
     * @return true if removed, false if not found
     */
    bool remove(const std::string& url);

    /**
     * @brief Check if URL is bookmarked
     */
    bool contains(const std::string& url) const;

    /**
     * @brief Get all bookmarks (sorted by timestamp, newest first)
     */
    std::vector<Bookmark> getAll() const;

    /**
     * @brief Clear all bookmarks
     */
    void clear();

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
