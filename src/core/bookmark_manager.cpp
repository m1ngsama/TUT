/**
 * @file bookmark_manager.cpp
 * @brief Bookmark manager implementation
 */

#include "core/bookmark_manager.hpp"
#include "utils/logger.hpp"
#include "utils/config.hpp"

#include <fstream>
#include <sstream>
#include <algorithm>
#include <chrono>
#include <sys/stat.h>
#include <sys/types.h>

namespace tut {

class BookmarkManager::Impl {
public:
    std::vector<Bookmark> bookmarks_;
    std::string filepath_;

    Impl() {
        // Get bookmark file path
        Config& config = Config::instance();
        std::string config_dir = config.getConfigPath();
        filepath_ = config_dir + "/bookmarks.json";

        // Ensure config directory exists
        mkdir(config_dir.c_str(), 0755);

        // Load existing bookmarks
        load();
    }

    void load() {
        std::ifstream file(filepath_);
        if (!file.is_open()) {
            LOG_DEBUG << "No bookmark file found, starting fresh";
            return;
        }

        bookmarks_.clear();
        std::string line;

        // Skip opening brace
        std::getline(file, line);

        while (std::getline(file, line)) {
            // Skip closing brace and empty lines
            if (line.find('}') != std::string::npos || line.empty()) {
                continue;
            }

            // Simple JSON parsing for bookmark entries
            // Format: {"title": "...", "url": "...", "timestamp": 123}
            if (line.find("\"title\"") != std::string::npos) {
                Bookmark bookmark;

                // Parse title
                size_t title_start = line.find("\"title\"") + 10;
                size_t title_end = line.find("\"", title_start);
                if (title_start != std::string::npos && title_end != std::string::npos) {
                    bookmark.title = line.substr(title_start, title_end - title_start);
                }

                // Parse URL
                size_t url_start = line.find("\"url\"") + 8;
                size_t url_end = line.find("\"", url_start);
                if (url_start != std::string::npos && url_end != std::string::npos) {
                    bookmark.url = line.substr(url_start, url_end - url_start);
                }

                // Parse timestamp
                size_t ts_start = line.find("\"timestamp\"") + 13;
                size_t ts_end = line.find_first_of(",}", ts_start);
                if (ts_start != std::string::npos && ts_end != std::string::npos) {
                    std::string ts_str = line.substr(ts_start, ts_end - ts_start);
                    try {
                        bookmark.timestamp = std::stoll(ts_str);
                    } catch (...) {
                        bookmark.timestamp = 0;
                    }
                }

                if (!bookmark.url.empty()) {
                    bookmarks_.push_back(bookmark);
                }
            }
        }

        LOG_INFO << "Loaded " << bookmarks_.size() << " bookmarks";
    }

    void save() {
        std::ofstream file(filepath_);
        if (!file.is_open()) {
            LOG_ERROR << "Failed to save bookmarks to " << filepath_;
            return;
        }

        file << "[\n";
        for (size_t i = 0; i < bookmarks_.size(); ++i) {
            const auto& bm = bookmarks_[i];
            file << "  {\"title\": \"" << escapeJson(bm.title)
                 << "\", \"url\": \"" << escapeJson(bm.url)
                 << "\", \"timestamp\": " << bm.timestamp << "}";
            if (i < bookmarks_.size() - 1) {
                file << ",";
            }
            file << "\n";
        }
        file << "]\n";

        LOG_DEBUG << "Saved " << bookmarks_.size() << " bookmarks";
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

BookmarkManager::BookmarkManager() : impl_(std::make_unique<Impl>()) {}

BookmarkManager::~BookmarkManager() = default;

bool BookmarkManager::add(const std::string& title, const std::string& url) {
    // Check if already exists
    if (contains(url)) {
        LOG_DEBUG << "Bookmark already exists: " << url;
        return false;
    }

    Bookmark bookmark(title, url, impl_->getCurrentTimestamp());
    impl_->bookmarks_.push_back(bookmark);
    impl_->save();

    LOG_INFO << "Added bookmark: " << title << " (" << url << ")";
    return true;
}

bool BookmarkManager::remove(const std::string& url) {
    auto it = std::find_if(impl_->bookmarks_.begin(), impl_->bookmarks_.end(),
                          [&url](const Bookmark& bm) { return bm.url == url; });

    if (it == impl_->bookmarks_.end()) {
        return false;
    }

    impl_->bookmarks_.erase(it);
    impl_->save();

    LOG_INFO << "Removed bookmark: " << url;
    return true;
}

bool BookmarkManager::contains(const std::string& url) const {
    return std::find_if(impl_->bookmarks_.begin(), impl_->bookmarks_.end(),
                       [&url](const Bookmark& bm) { return bm.url == url; }) !=
           impl_->bookmarks_.end();
}

std::vector<Bookmark> BookmarkManager::getAll() const {
    // Return sorted by timestamp (newest first)
    std::vector<Bookmark> result = impl_->bookmarks_;
    std::sort(result.begin(), result.end(),
             [](const Bookmark& a, const Bookmark& b) {
                 return a.timestamp > b.timestamp;
             });
    return result;
}

void BookmarkManager::clear() {
    impl_->bookmarks_.clear();
    impl_->save();
    LOG_INFO << "Cleared all bookmarks";
}

}  // namespace tut
