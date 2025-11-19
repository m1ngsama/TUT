#include "ics_parser.h"

#include <algorithm>
#include <cctype>
#include <iomanip>
#include <optional>
#include <sstream>
#include <stdexcept>

namespace {

// 去掉首尾空白
std::string trim(const std::string &s) {
    size_t start = 0;
    while (start < s.size() && std::isspace(static_cast<unsigned char>(s[start]))) ++start;
    size_t end = s.size();
    while (end > start && std::isspace(static_cast<unsigned char>(s[end - 1]))) --end;
    return s.substr(start, end - start);
}

// 将 ICS 中换行折叠处理（以空格或 Tab 开头的行拼接到前一行）
std::vector<std::string> unfold_lines(const std::string &text) {
    std::vector<std::string> lines;
    std::string current;
    std::istringstream iss(text);
    std::string line;
    while (std::getline(iss, line)) {
        if (!line.empty() && (line.back() == '\r' || line.back() == '\n')) {
            line.pop_back();
        }
        if (!line.empty() && (line[0] == ' ' || line[0] == '\t')) {
            // continuation
            current += trim(line);
        } else {
            if (!current.empty()) {
                lines.push_back(current);
            }
            current = line;
        }
    }
    if (!current.empty()) {
        lines.push_back(current);
    }
    return lines;
}

// 仅支持几种常见格式：YYYYMMDD 或 YYYYMMDDTHHMMSSZ / 本地时间
std::chrono::system_clock::time_point parse_ics_datetime(const std::string &value) {
    std::tm tm{};
    if (value.size() == 8) {
        // 日期
        std::istringstream ss(value);
        ss >> std::get_time(&tm, "%Y%m%d");
        if (ss.fail()) {
            throw std::runtime_error("无法解析日期: " + value);
        }
        tm.tm_hour = 0;
        tm.tm_min = 0;
        tm.tm_sec = 0;
    } else if (value.size() >= 15 && value[8] == 'T') {
        // 日期时间，如 20250101T090000Z 或无 Z
        std::string fmt = "%Y%m%dT%H%M%S";
        std::string v = value;
        bool hasZ = false;
        if (!v.empty() && v.back() == 'Z') {
            hasZ = true;
            v.pop_back();
        }
        std::istringstream ss(v);
        ss >> std::get_time(&tm, fmt.c_str());
        if (ss.fail()) {
            throw std::runtime_error("无法解析日期时间: " + value);
        }
        // 这里简单按本地时间处理；如需严格 UTC 可改用 timegm
    } else {
        throw std::runtime_error("未知日期格式: " + value);
    }

    std::time_t t = std::mktime(&tm);
    return std::chrono::system_clock::from_time_t(t);
}

std::string get_prop_value(const std::string &line) {
    auto pos = line.find(':');
    if (pos == std::string::npos) return {};
    return line.substr(pos + 1);
}

// 获取属性参数和值，例如 "DTSTART;TZID=Asia/Shanghai:20251121T203000"
// 返回 "DTSTART;TZID=Asia/Shanghai" 作为 key，冒号后的部分作为 value。
std::pair<std::string, std::string> split_prop(const std::string &line) {
    auto pos = line.find(':');
    if (pos == std::string::npos) return {line, {}};
    return {line.substr(0, pos), line.substr(pos + 1)};
}

// 从 RRULE 中提取 UNTIL=... 的值（若存在）
std::optional<std::string> extract_until_str(const std::string &rrule) {
    // 例：RRULE:FREQ=WEEKLY;BYDAY=FR;UNTIL=20260401T000000Z
    auto pos = rrule.find("UNTIL=");
    if (pos == std::string::npos) return std::nullopt;
    pos += 6; // 跳过 "UNTIL="
    size_t end = rrule.find(';', pos);
    if (end == std::string::npos) end = rrule.size();
    if (pos >= rrule.size()) return std::nullopt;
    return rrule.substr(pos, end - pos);
}

bool starts_with(const std::string &s, const std::string &prefix) {
    return s.size() >= prefix.size() && std::equal(prefix.begin(), prefix.end(), s.begin());
}

} // namespace

std::vector<IcsEvent> parse_ics(const std::string &icsText) {
    auto lines = unfold_lines(icsText);
    std::vector<IcsEvent> events;

    bool inEvent = false;
    IcsEvent current{};

    for (const auto &rawLine : lines) {
        std::string line = trim(rawLine);
        if (line == "BEGIN:VEVENT") {
            inEvent = true;
            current = IcsEvent{};
        } else if (line == "END:VEVENT") {
            if (inEvent) {
                events.push_back(current);
            }
            inEvent = false;
        } else if (inEvent) {
            if (starts_with(line, "DTSTART")) {
                auto [key, v] = split_prop(line);
                current.start = parse_ics_datetime(v);
            } else if (starts_with(line, "DTEND")) {
                auto [key, v] = split_prop(line);
                current.end = parse_ics_datetime(v);
            } else if (starts_with(line, "SUMMARY")) {
                current.summary = get_prop_value(line);
            } else if (starts_with(line, "LOCATION")) {
                current.location = get_prop_value(line);
            } else if (starts_with(line, "DESCRIPTION")) {
                current.description = get_prop_value(line);
            } else if (starts_with(line, "RRULE")) {
                current.rrule = get_prop_value(line);
                if (auto untilStr = extract_until_str(current.rrule)) {
                    try {
                        current.until = parse_ics_datetime(*untilStr);
                    } catch (...) {
                        // UNTIL 解析失败时忽略截止时间，照常视为无限期
                    }
                }
            }
        }
    }

    // 按开始时间排序
    std::sort(events.begin(), events.end(),
              [](const IcsEvent &a, const IcsEvent &b) { return a.start < b.start; });

    return events;
}


