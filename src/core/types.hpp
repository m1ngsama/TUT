/**
 * @file types.hpp
 * @brief Common types used across TUT modules
 * @author m1ngsama
 * @date 2024-12-31
 */

#pragma once

#include <string>

namespace tut {

/**
 * @brief 链接信息结构体
 */
struct LinkInfo {
    std::string url;    ///< 链接 URL
    std::string text;   ///< 链接文本
    int line{0};        ///< 所在行号
};

}  // namespace tut
