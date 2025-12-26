#pragma once

#include <cstdint>

namespace tut {

/**
 * 颜色定义 - True Color (24-bit RGB)
 *
 * 使用温暖的配色方案，适合长时间阅读
 */
namespace colors {

// ==================== 基础颜色 ====================

// 背景色
constexpr uint32_t BG_PRIMARY    = 0x1A1A1A;  // 主背景 - 深灰
constexpr uint32_t BG_SECONDARY  = 0x252525;  // 次背景 - 稍浅灰
constexpr uint32_t BG_ELEVATED   = 0x2A2A2A;  // 抬升背景 - 用于卡片/区块
constexpr uint32_t BG_SELECTION  = 0x3A3A3A;  // 选中背景

// 前景色
constexpr uint32_t FG_PRIMARY    = 0xD0D0D0;  // 主文本 - 浅灰
constexpr uint32_t FG_SECONDARY  = 0x909090;  // 次文本 - 中灰
constexpr uint32_t FG_DIM        = 0x606060;  // 暗淡文本

// ==================== 语义颜色 ====================

// 标题
constexpr uint32_t H1_FG         = 0xE8C48C;  // H1 - 暖金色
constexpr uint32_t H2_FG         = 0x88C0D0;  // H2 - 冰蓝色
constexpr uint32_t H3_FG         = 0xA3BE8C;  // H3 - 柔绿色

// 链接
constexpr uint32_t LINK_FG       = 0x81A1C1;  // 链接 - 柔蓝色
constexpr uint32_t LINK_ACTIVE   = 0x88C0D0;  // 活跃链接 - 亮蓝色
constexpr uint32_t LINK_VISITED  = 0xB48EAD;  // 已访问链接 - 柔紫色

// 表单元素
constexpr uint32_t INPUT_BG      = 0x2E3440;  // 输入框背景
constexpr uint32_t INPUT_BORDER  = 0x4C566A;  // 输入框边框
constexpr uint32_t INPUT_FOCUS   = 0x5E81AC;  // 聚焦边框

// 状态颜色
constexpr uint32_t SUCCESS       = 0xA3BE8C;  // 成功 - 绿色
constexpr uint32_t WARNING       = 0xEBCB8B;  // 警告 - 黄色
constexpr uint32_t ERROR         = 0xBF616A;  // 错误 - 红色
constexpr uint32_t INFO          = 0x88C0D0;  // 信息 - 蓝色

// ==================== UI元素颜色 ====================

// 状态栏
constexpr uint32_t STATUSBAR_BG  = 0x2E3440;  // 状态栏背景
constexpr uint32_t STATUSBAR_FG  = 0xD8DEE9;  // 状态栏文本

// URL栏
constexpr uint32_t URLBAR_BG     = 0x3B4252;  // URL栏背景
constexpr uint32_t URLBAR_FG     = 0xECEFF4;  // URL栏文本

// 搜索高亮
constexpr uint32_t SEARCH_MATCH_BG = 0x4C566A;
constexpr uint32_t SEARCH_MATCH_FG = 0xECEFF4;
constexpr uint32_t SEARCH_CURRENT_BG = 0x5E81AC;
constexpr uint32_t SEARCH_CURRENT_FG = 0xFFFFFF;

// 装饰元素
constexpr uint32_t BORDER        = 0x4C566A;  // 边框
constexpr uint32_t DIVIDER       = 0x3B4252;  // 分隔线

// 代码块
constexpr uint32_t CODE_BG       = 0x2E3440;  // 代码背景
constexpr uint32_t CODE_FG       = 0xD8DEE9;  // 代码文本

// 引用块
constexpr uint32_t QUOTE_BORDER  = 0x4C566A;  // 引用边框
constexpr uint32_t QUOTE_FG      = 0x909090;  // 引用文本

// 表格
constexpr uint32_t TABLE_BORDER  = 0x4C566A;
constexpr uint32_t TABLE_HEADER_BG = 0x2E3440;
constexpr uint32_t TABLE_ROW_ALT = 0x252525;  // 交替行

} // namespace colors

/**
 * RGB辅助函数
 */
inline constexpr uint32_t rgb(uint8_t r, uint8_t g, uint8_t b) {
    return (static_cast<uint32_t>(r) << 16) |
           (static_cast<uint32_t>(g) << 8) |
           static_cast<uint32_t>(b);
}

inline constexpr uint8_t get_red(uint32_t color) {
    return (color >> 16) & 0xFF;
}

inline constexpr uint8_t get_green(uint32_t color) {
    return (color >> 8) & 0xFF;
}

inline constexpr uint8_t get_blue(uint32_t color) {
    return color & 0xFF;
}

/**
 * 颜色混合（线性插值）
 */
inline uint32_t blend_colors(uint32_t c1, uint32_t c2, float t) {
    uint8_t r = static_cast<uint8_t>(get_red(c1) * (1 - t) + get_red(c2) * t);
    uint8_t g = static_cast<uint8_t>(get_green(c1) * (1 - t) + get_green(c2) * t);
    uint8_t b = static_cast<uint8_t>(get_blue(c1) * (1 - t) + get_blue(c2) * t);
    return rgb(r, g, b);
}

} // namespace tut
