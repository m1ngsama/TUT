#include "history.h"
#include <fstream>
#include <sstream>
#include <algorithm>
#include <sys/stat.h>
#include <cstdlib>

namespace tut {

HistoryManager::HistoryManager() {
    load();
}

HistoryManager::~HistoryManager() {
    save();
}

std::string HistoryManager::get_history_path() {
    const char* home = std::getenv("HOME");
    if (!home) {
        home = "/tmp";
    }
    return std::string(home) + "/.config/tut/history.json";
}

bool HistoryManager::ensure_config_dir() {
    const char* home = std::getenv("HOME");
    if (!home) home = "/tmp";

    std::string config_dir = std::string(home) + "/.config";
    std::string tut_dir = config_dir + "/tut";

    struct stat st;
    if (stat(tut_dir.c_str(), &st) == 0) {
        return S_ISDIR(st.st_mode);
    }

    mkdir(config_dir.c_str(), 0755);
    return mkdir(tut_dir.c_str(), 0755) == 0 || errno == EEXIST;
}

// JSON escape/unescape
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

bool HistoryManager::load() {
    entries_.clear();

    std::ifstream file(get_history_path());
    if (!file) {
        return false;
    }

    std::string content((std::istreambuf_iterator<char>(file)),
                        std::istreambuf_iterator<char>());
    file.close();

    size_t pos = content.find('[');
    if (pos == std::string::npos) return false;
    pos++;

    while (pos < content.size()) {
        pos = content.find('{', pos);
        if (pos == std::string::npos) break;
        pos++;

        HistoryEntry entry;

        while (pos < content.size() && content[pos] != '}') {
            while (pos < content.size() && (content[pos] == ' ' || content[pos] == '\n' ||
                   content[pos] == '\r' || content[pos] == '\t' || content[pos] == ',')) {
                pos++;
            }

            if (content[pos] == '}') break;

            if (content[pos] != '"') { pos++; continue; }
            pos++;

            size_t key_end = content.find('"', pos);
            if (key_end == std::string::npos) break;
            std::string key = content.substr(pos, key_end - pos);
            pos = key_end + 1;

            pos = content.find(':', pos);
            if (pos == std::string::npos) break;
            pos++;

            while (pos < content.size() && (content[pos] == ' ' || content[pos] == '\n' ||
                   content[pos] == '\r' || content[pos] == '\t')) {
                pos++;
            }

            if (content[pos] == '"') {
                pos++;
                size_t val_end = pos;
                while (val_end < content.size()) {
                    if (content[val_end] == '"' && content[val_end - 1] != '\\') break;
                    val_end++;
                }
                std::string value = json_unescape(content.substr(pos, val_end - pos));
                pos = val_end + 1;

                if (key == "url") entry.url = value;
                else if (key == "title") entry.title = value;
            } else {
                size_t val_end = pos;
                while (val_end < content.size() && content[val_end] >= '0' && content[val_end] <= '9') {
                    val_end++;
                }
                std::string value = content.substr(pos, val_end - pos);
                pos = val_end;

                if (key == "time") {
                    entry.visit_time = std::stoll(value);
                }
            }
        }

        if (!entry.url.empty()) {
            entries_.push_back(entry);
        }

        pos = content.find('}', pos);
        if (pos == std::string::npos) break;
        pos++;
    }

    return true;
}

bool HistoryManager::save() const {
    if (!ensure_config_dir()) {
        return false;
    }

    std::ofstream file(get_history_path());
    if (!file) {
        return false;
    }

    file << "[\n";
    for (size_t i = 0; i < entries_.size(); ++i) {
        const auto& entry = entries_[i];
        file << "  {\n";
        file << "    \"url\": \"" << json_escape(entry.url) << "\",\n";
        file << "    \"title\": \"" << json_escape(entry.title) << "\",\n";
        file << "    \"time\": " << entry.visit_time << "\n";
        file << "  }";
        if (i + 1 < entries_.size()) {
            file << ",";
        }
        file << "\n";
    }
    file << "]\n";

    return true;
}

void HistoryManager::add(const std::string& url, const std::string& title) {
    // Remove existing entry with same URL
    auto it = std::find_if(entries_.begin(), entries_.end(),
                           [&url](const HistoryEntry& e) { return e.url == url; });
    if (it != entries_.end()) {
        entries_.erase(it);
    }

    // Add new entry at the front
    entries_.insert(entries_.begin(), HistoryEntry(url, title));

    // Enforce max entries limit
    if (entries_.size() > MAX_ENTRIES) {
        entries_.resize(MAX_ENTRIES);
    }

    save();
}

void HistoryManager::clear() {
    entries_.clear();
    save();
}

} // namespace tut
