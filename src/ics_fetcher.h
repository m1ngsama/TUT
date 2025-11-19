#pragma once

#include <string>

// 从给定 URL 获取 ICS 文本，失败抛出 std::runtime_error
std::string fetch_ics(const std::string &url);


