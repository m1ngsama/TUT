#pragma once

#include <chrono>
#include <optional>
#include <string>
#include <vector>

struct IcsEvent {
    std::chrono::system_clock::time_point start;
    std::chrono::system_clock::time_point end;
    std::string summary;
    std::string location;
    std::string description;

    // 简单递归支持：保留 RRULE 原文，以及可选的 UNTIL 截止时间（若存在）
    std::string rrule;
    std::optional<std::chrono::system_clock::time_point> until;
};

// 解析 ICS 文本，返回“基准事件”（不展开 RRULE）。
// 周期事件会在 IcsEvent::rrule 与 IcsEvent::until 中体现。
std::vector<IcsEvent> parse_ics(const std::string &icsText);


