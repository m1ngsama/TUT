#include "bookmark.h"
#include <fstream>
#include <sstream>
#include <algorithm>
#include <sys/stat.h>
#include <cstdlib>

namespace tut {

BookmarkManager::BookmarkManager() {
    load();
}

BookmarkManager::~BookmarkManager() {
    save();
}

std::string BookmarkManager::get_config_dir() {
    const char* home = std::getenv("HOME");
    if (!home) {
        home = "/tmp";
    }
    return std::string(home) + "/.config/tut";
}

std::string BookmarkManager::get_bookmarks_path() {
    return get_config_dir() + "/bookmarks.json";
}

bool BookmarkManager::ensure_config_dir() {
    std::string dir = get_config_dir();

    // 检查目录是否存在
    struct stat st;
    if (stat(dir.c_str(), &st) == 0) {
        return S_ISDIR(st.st_mode);
    }

    // 创建 ~/.config 目录
    std::string config_dir = std::string(std::getenv("HOME") ? std::getenv("HOME") : "/tmp") + "/.config";
    mkdir(config_dir.c_str(), 0755);

    // 创建 ~/.config/tut 目录
    return mkdir(dir.c_str(), 0755) == 0 || errno == EEXIST;
}

// 简单的 JSON 转义
static std::string json_escape(const std::string& s) {
    std::string result;
    result.reserve(s.size() + 10);
    for (char c : s) {
        switch (c) {
            case '"': result += "\\\""; break;
            case '\\': result += "\\\\"; break;
            case '\n': result += "\\n"; break;
            case '\r': result += "\\r"; break;
            case '\t': result += "\\t"; break;
            default: result += c; break;
        }
    }
    return result;
}

// 简单的 JSON 反转义
static std::string json_unescape(const std::string& s) {
    std::string result;
    result.reserve(s.size());
    for (size_t i = 0; i < s.size(); ++i) {
        if (s[i] == '\\' && i + 1 < s.size()) {
            switch (s[i + 1]) {
                case '"': result += '"'; ++i; break;
                case '\\': result += '\\'; ++i; break;
                case 'n': result += '\n'; ++i; break;
                case 'r': result += '\r'; ++i; break;
                case 't': result += '\t'; ++i; break;
                default: result += s[i]; break;
            }
        } else {
            result += s[i];
        }
    }
    return result;
}

bool BookmarkManager::load() {
    bookmarks_.clear();

    std::ifstream file(get_bookmarks_path());
    if (!file) {
        return false;  // 文件不存在，这是正常的
    }

    std::string content((std::istreambuf_iterator<char>(file)),
                        std::istreambuf_iterator<char>());
    file.close();

    // 简单的 JSON 解析
    // 格式: [{"url":"...","title":"...","time":123}, ...]
    size_t pos = content.find('[');
    if (pos == std::string::npos) return false;

    pos++;  // 跳过 '['

    while (pos < content.size()) {
        // 查找对象开始
        pos = content.find('{', pos);
        if (pos == std::string::npos) break;
        pos++;

        Bookmark bm;

        // 解析字段
        while (pos < content.size() && content[pos] != '}') {
            // 跳过空白
            while (pos < content.size() && (content[pos] == ' ' || content[pos] == '\n' ||
                   content[pos] == '\r' || content[pos] == '\t' || content[pos] == ',')) {
                pos++;
            }

            if (content[pos] == '}') break;

            // 读取键名
            if (content[pos] != '"') { pos++; continue; }
            pos++;  // 跳过 '"'

            size_t key_end = content.find('"', pos);
            if (key_end == std::string::npos) break;
            std::string key = content.substr(pos, key_end - pos);
            pos = key_end + 1;

            // 跳过 ':'
            pos = content.find(':', pos);
            if (pos == std::string::npos) break;
            pos++;

            // 跳过空白
            while (pos < content.size() && (content[pos] == ' ' || content[pos] == '\n' ||
                   content[pos] == '\r' || content[pos] == '\t')) {
                pos++;
            }

            if (content[pos] == '"') {
                // 字符串值
                pos++;  // 跳过 '"'
                size_t val_end = pos;
                while (val_end < content.size()) {
                    if (content[val_end] == '"' && content[val_end - 1] != '\\') break;
                    val_end++;
                }
                std::string value = json_unescape(content.substr(pos, val_end - pos));
                pos = val_end + 1;

                if (key == "url") bm.url = value;
                else if (key == "title") bm.title = value;
            } else {
                // 数字值
                size_t val_end = pos;
                while (val_end < content.size() && content[val_end] >= '0' && content[val_end] <= '9') {
                    val_end++;
                }
                std::string value = content.substr(pos, val_end - pos);
                pos = val_end;

                if (key == "time") {
                    bm.added_time = std::stoll(value);
                }
            }
        }

        if (!bm.url.empty()) {
            bookmarks_.push_back(bm);
        }

        // 跳到下一个对象
        pos = content.find('}', pos);
        if (pos == std::string::npos) break;
        pos++;
    }

    return true;
}

bool BookmarkManager::save() const {
    if (!ensure_config_dir()) {
        return false;
    }

    std::ofstream file(get_bookmarks_path());
    if (!file) {
        return false;
    }

    file << "[\n";
    for (size_t i = 0; i < bookmarks_.size(); ++i) {
        const auto& bm = bookmarks_[i];
        file << "  {\n";
        file << "    \"url\": \"" << json_escape(bm.url) << "\",\n";
        file << "    \"title\": \"" << json_escape(bm.title) << "\",\n";
        file << "    \"time\": " << bm.added_time << "\n";
        file << "  }";
        if (i + 1 < bookmarks_.size()) {
            file << ",";
        }
        file << "\n";
    }
    file << "]\n";

    return true;
}

bool BookmarkManager::add(const std::string& url, const std::string& title) {
    // 检查是否已存在
    if (contains(url)) {
        return false;
    }

    bookmarks_.emplace_back(url, title);
    return save();
}

bool BookmarkManager::remove(const std::string& url) {
    auto it = std::find_if(bookmarks_.begin(), bookmarks_.end(),
                           [&url](const Bookmark& bm) { return bm.url == url; });

    if (it == bookmarks_.end()) {
        return false;
    }

    bookmarks_.erase(it);
    return save();
}

bool BookmarkManager::remove_at(size_t index) {
    if (index >= bookmarks_.size()) {
        return false;
    }

    bookmarks_.erase(bookmarks_.begin() + index);
    return save();
}

bool BookmarkManager::contains(const std::string& url) const {
    return std::find_if(bookmarks_.begin(), bookmarks_.end(),
                        [&url](const Bookmark& bm) { return bm.url == url; })
           != bookmarks_.end();
}

} // namespace tut
