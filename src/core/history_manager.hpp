/**
 * @file history_manager.hpp
 * @brief History manager for persistent browsing history
 * @author m1ngsama
 * @date 2025-01-01
 */

#pragma once

#include <string>
#include <vector>
#include <memory>

namespace tut {

/**
 * @brief History entry
 */
struct HistoryEntry {
    std::string title;
    std::string url;
    int64_t timestamp{0};  // Unix timestamp of last visit

    HistoryEntry() = default;
    HistoryEntry(const std::string& t, const std::string& u, int64_t ts = 0)
        : title(t), url(u), timestamp(ts) {}
};

/**
 * @brief History manager with JSON persistence
 *
 * Manages browsing history with automatic persistence to
 * ~/.config/tut/history.json
 *
 * Features:
 * - Auto-records page visits
 * - Updates timestamp on revisit (moves to front)
 * - Limits to max 1000 entries
 */
class HistoryManager {
public:
    HistoryManager();
    ~HistoryManager();

    /**
     * @brief Record a page visit
     * If URL exists, updates timestamp and moves to front
     * @param title Page title
     * @param url Page URL
     */
    void recordVisit(const std::string& title, const std::string& url);

    /**
     * @brief Get all history entries (sorted by timestamp, newest first)
     */
    std::vector<HistoryEntry> getAll() const;

    /**
     * @brief Get recent history (last N entries)
     */
    std::vector<HistoryEntry> getRecent(int count) const;

    /**
     * @brief Clear all history
     */
    void clear();

    /**
     * @brief Get total number of history entries
     */
    size_t size() const;

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
