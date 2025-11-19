#include "calendar.h"
#include <iostream>
#include <vector>
#include <string>
#include <chrono>
#include <algorithm>

#include "ics_fetcher.h"
#include "ics_parser.h"
#include "tui_view.h"

void Calendar::run() {
    try {
        // 1. 获取 ICS 文本
        std::string url = "https://ical.nbtca.space/nbtca.ics";
        std::string icsData = fetch_ics(url);

        // 2. 解析事件
        auto allEvents = parse_ics(icsData);

        // 3. 过滤未来一个月的事件（支持简单的每周 RRULE）
        auto now = std::chrono::system_clock::now();
        auto oneMonthLater = now + std::chrono::hours(24 * 30);

        std::vector<IcsEvent> upcoming;
        for (const auto &ev : allEvents) {
            // 简单处理：如果包含 FREQ=WEEKLY，则视为每周重复事件
            if (ev.rrule.find("FREQ=WEEKLY") != std::string::npos) {
                // 从基准时间往后每 7 天生成一次，直到超过 oneMonthLater 或 UNTIL
                auto curStart = ev.start;
                auto curEnd = ev.end;

                // 如果有 UNTIL，则作为上界
                auto upper = oneMonthLater;
                if (ev.until && *ev.until < upper) {
                    upper = *ev.until;
                }

                // 为避免意外死循环，限制最多展开约 10 年（~520 周）
                const int maxIterations = 520;
                int iter = 0;

                while (curStart <= upper && iter < maxIterations) {
                    if (curStart >= now && curStart <= upper) {
                        IcsEvent occ = ev;
                        occ.start = curStart;
                        occ.end = curEnd;
                        upcoming.push_back(std::move(occ));
                    }
                    curStart += std::chrono::hours(24 * 7);
                    curEnd += std::chrono::hours(24 * 7);
                    ++iter;
                }
            } else {
                // 非 RRULE 事件：直接按时间窗口筛选
                if (ev.start >= now && ev.start <= oneMonthLater) {
                    upcoming.push_back(ev);
                }
            }
        }

        // 确保展示按时间排序
        std::sort(upcoming.begin(), upcoming.end(),
                  [](const IcsEvent &a, const IcsEvent &b) { return a.start < b.start; });

        // 4. 启动 TUI 展示（只展示未来一个月的活动）
        run_tui(upcoming);
    } catch (const std::exception &ex) {
        std::cerr << "错误: " << ex.what() << std::endl;
    }
}
