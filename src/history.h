#pragma once

#include <string>
#include <vector>
#include <ctime>

namespace tut {

/**
 * 历史记录条目
 */
struct HistoryEntry {
    std::string url;
    std::string title;
    std::time_t visit_time;

    HistoryEntry() : visit_time(0) {}
    HistoryEntry(const std::string& url, const std::string& title)
        : url(url), title(title), visit_time(std::time(nullptr)) {}
};

/**
 * 历史记录管理器
 *
 * 历史记录存储在 ~/.config/tut/history.json
 * 最多保存 MAX_ENTRIES 条记录
 */
class HistoryManager {
public:
    static constexpr size_t MAX_ENTRIES = 1000;

    HistoryManager();
    ~HistoryManager();

    /**
     * 加载历史记录
     */
    bool load();

    /**
     * 保存历史记录
     */
    bool save() const;

    /**
     * 添加历史记录
     * 如果 URL 已存在，会更新访问时间并移到最前面
     */
    void add(const std::string& url, const std::string& title);

    /**
     * 清空历史记录
     */
    void clear();

    /**
     * 获取历史记录列表（最新的在前面）
     */
    const std::vector<HistoryEntry>& get_all() const { return entries_; }

    /**
     * 获取历史记录数量
     */
    size_t count() const { return entries_.size(); }

    /**
     * 获取历史记录文件路径
     */
    static std::string get_history_path();

private:
    std::vector<HistoryEntry> entries_;

    // 确保配置目录存在
    static bool ensure_config_dir();
};

} // namespace tut
