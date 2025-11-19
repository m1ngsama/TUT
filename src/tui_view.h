#pragma once

#include "ics_parser.h"
#include <vector>

// 运行 ncurses TUI，展示给定事件列表
// 当 events 为空时，在界面上提示“未来一个月暂无活动”
void run_tui(const std::vector<IcsEvent> &events);


