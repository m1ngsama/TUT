#pragma once

#include "terminal.h"
#include <vector>
#include <string>
#include <cstdint>

namespace tut {

/**
 * 文本属性位标志
 */
enum CellAttr : uint8_t {
    ATTR_NONE      = 0,
    ATTR_BOLD      = 1 << 0,
    ATTR_ITALIC    = 1 << 1,
    ATTR_UNDERLINE = 1 << 2,
    ATTR_REVERSE   = 1 << 3,
    ATTR_DIM       = 1 << 4
};

/**
 * Cell - 单个字符单元格
 *
 * 存储一个UTF-8字符及其渲染属性
 */
struct Cell {
    std::string content;     // UTF-8字符（可能1-4字节）
    uint32_t fg = 0xD0D0D0;  // 前景色 (默认浅灰)
    uint32_t bg = 0x1A1A1A;  // 背景色 (默认深灰)
    uint8_t attrs = ATTR_NONE;

    bool operator==(const Cell& other) const {
        return content == other.content &&
               fg == other.fg &&
               bg == other.bg &&
               attrs == other.attrs;
    }

    bool operator!=(const Cell& other) const {
        return !(*this == other);
    }
};

/**
 * FrameBuffer - 帧缓冲区
 *
 * 双缓冲渲染：维护当前帧和上一帧，只渲染变化的部分
 */
class FrameBuffer {
public:
    FrameBuffer(int width, int height);

    void resize(int width, int height);
    void clear();
    void clear_with_color(uint32_t bg);

    void set_cell(int x, int y, const Cell& cell);
    const Cell& get_cell(int x, int y) const;

    // 便捷方法：设置文本（处理宽字符）
    void set_text(int x, int y, const std::string& text, uint32_t fg, uint32_t bg, uint8_t attrs = ATTR_NONE);

    int width() const { return width_; }
    int height() const { return height_; }

private:
    std::vector<std::vector<Cell>> cells_;
    int width_;
    int height_;
    Cell empty_cell_;
};

/**
 * Renderer - 渲染器
 *
 * 负责将FrameBuffer的内容渲染到终端
 * 实现差分渲染以提高性能
 */
class Renderer {
public:
    explicit Renderer(Terminal& terminal);

    /**
     * 渲染帧缓冲区到终端
     * 使用差分算法只更新变化的部分
     */
    void render(const FrameBuffer& buffer);

    /**
     * 强制全屏重绘
     */
    void force_redraw();

private:
    Terminal& terminal_;
    FrameBuffer prev_buffer_;  // 上一帧，用于差分渲染
    bool need_full_redraw_ = true;

    void apply_cell_style(const Cell& cell);
};

} // namespace tut
