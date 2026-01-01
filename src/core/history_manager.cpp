/**
 * @file history_manager.cpp
 * @brief History manager implementation
 */

#include "core/history_manager.hpp"
#include "utils/logger.hpp"
#include "utils/config.hpp"

#include <fstream>
#include <sstream>
#include <algorithm>
#include <chrono>
#include <sys/stat.h>
#include <sys/types.h>

namespace tut {

class HistoryManager::Impl {
public:
    std::vector<HistoryEntry> entries_;
    std::string filepath_;
    static constexpr size_t MAX_ENTRIES = 1000;

    Impl() {
        // Get history file path
        Config& config = Config::instance();
        std::string config_dir = config.getConfigPath();
        filepath_ = config_dir + "/history.json";

        // Ensure config directory exists
        mkdir(config_dir.c_str(), 0755);

        // Load existing history
        load();
    }

    void load() {
        std::ifstream file(filepath_);
        if (!file.is_open()) {
            LOG_DEBUG << "No history file found, starting fresh";
            return;
        }

        entries_.clear();
        std::string line;

        // Skip opening brace
        std::getline(file, line);

        while (std::getline(file, line)) {
            // Skip closing brace and empty lines
            if (line.find('}') != std::string::npos || line.empty()) {
                continue;
            }

            // Simple JSON parsing for history entries
            // Format: {"title": "...", "url": "...", "timestamp": 123}
            if (line.find("\"title\"") != std::string::npos) {
                HistoryEntry entry;

                // Parse title
                size_t title_start = line.find("\"title\"") + 10;
                size_t title_end = line.find("\"", title_start);
                if (title_start != std::string::npos && title_end != std::string::npos) {
                    entry.title = line.substr(title_start, title_end - title_start);
                }

                // Parse URL
                size_t url_start = line.find("\"url\"") + 8;
                size_t url_end = line.find("\"", url_start);
                if (url_start != std::string::npos && url_end != std::string::npos) {
                    entry.url = line.substr(url_start, url_end - url_start);
                }

                // Parse timestamp
                size_t ts_start = line.find("\"timestamp\"") + 13;
                size_t ts_end = line.find_first_of(",}", ts_start);
                if (ts_start != std::string::npos && ts_end != std::string::npos) {
                    std::string ts_str = line.substr(ts_start, ts_end - ts_start);
                    try {
                        entry.timestamp = std::stoll(ts_str);
                    } catch (...) {
                        entry.timestamp = 0;
                    }
                }

                if (!entry.url.empty()) {
                    entries_.push_back(entry);
                }
            }
        }

        LOG_INFO << "Loaded " << entries_.size() << " history entries";
    }

    void save() {
        std::ofstream file(filepath_);
        if (!file.is_open()) {
            LOG_ERROR << "Failed to save history to " << filepath_;
            return;
        }

        file << "[\n";
        for (size_t i = 0; i < entries_.size(); ++i) {
            const auto& entry = entries_[i];
            file << "  {\"title\": \"" << escapeJson(entry.title)
                 << "\", \"url\": \"" << escapeJson(entry.url)
                 << "\", \"timestamp\": " << entry.timestamp << "}";
            if (i < entries_.size() - 1) {
                file << ",";
            }
            file << "\n";
        }
        file << "]\n";

        LOG_DEBUG << "Saved " << entries_.size() << " history entries";
    }

    std::string escapeJson(const std::string& str) {
        std::string result;
        for (char c : str) {
            if (c == '"') {
                result += "\\\"";
            } else if (c == '\\') {
                result += "\\\\";
            } else if (c == '\n') {
                result += "\\n";
            } else if (c == '\t') {
                result += "\\t";
            } else {
                result += c;
            }
        }
        return result;
    }

    int64_t getCurrentTimestamp() {
        auto now = std::chrono::system_clock::now();
        auto duration = now.time_since_epoch();
        return std::chrono::duration_cast<std::chrono::seconds>(duration).count();
    }
};

HistoryManager::HistoryManager() : impl_(std::make_unique<Impl>()) {}

HistoryManager::~HistoryManager() = default;

void HistoryManager::recordVisit(const std::string& title, const std::string& url) {
    // Skip empty URLs or about:blank
    if (url.empty() || url == "about:blank") {
        return;
    }

    // Check if URL already exists
    auto it = std::find_if(impl_->entries_.begin(), impl_->entries_.end(),
                          [&url](const HistoryEntry& e) { return e.url == url; });

    if (it != impl_->entries_.end()) {
        // Update existing entry: update timestamp and move to front
        it->timestamp = impl_->getCurrentTimestamp();
        it->title = title;  // Update title too in case it changed

        // Move to front (most recent)
        HistoryEntry entry = *it;
        impl_->entries_.erase(it);
        impl_->entries_.insert(impl_->entries_.begin(), entry);

        LOG_DEBUG << "Updated history: " << title << " (" << url << ")";
    } else {
        // Add new entry at front
        HistoryEntry entry(title, url, impl_->getCurrentTimestamp());
        impl_->entries_.insert(impl_->entries_.begin(), entry);

        LOG_INFO << "Added to history: " << title << " (" << url << ")";

        // Enforce max entries limit
        if (impl_->entries_.size() > Impl::MAX_ENTRIES) {
            impl_->entries_.resize(Impl::MAX_ENTRIES);
            LOG_DEBUG << "Trimmed history to " << Impl::MAX_ENTRIES << " entries";
        }
    }

    impl_->save();
}

std::vector<HistoryEntry> HistoryManager::getAll() const {
    return impl_->entries_;  // Already sorted (newest first)
}

std::vector<HistoryEntry> HistoryManager::getRecent(int count) const {
    if (count <= 0 || impl_->entries_.empty()) {
        return {};
    }

    size_t n = std::min(static_cast<size_t>(count), impl_->entries_.size());
    return std::vector<HistoryEntry>(impl_->entries_.begin(),
                                     impl_->entries_.begin() + n);
}

void HistoryManager::clear() {
    impl_->entries_.clear();
    impl_->save();
    LOG_INFO << "Cleared all history";
}

size_t HistoryManager::size() const {
    return impl_->entries_.size();
}

}  // namespace tut
