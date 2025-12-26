#include "renderer.h"
#include "../utils/unicode.h"

namespace tut {

// ============================================================================
// FrameBuffer Implementation
// ============================================================================

FrameBuffer::FrameBuffer(int width, int height)
    : width_(width), height_(height) {
    empty_cell_.content = " ";
    resize(width, height);
}

void FrameBuffer::resize(int width, int height) {
    width_ = width;
    height_ = height;
    cells_.resize(height);
    for (auto& row : cells_) {
        row.resize(width, empty_cell_);
    }
}

void FrameBuffer::clear() {
    for (auto& row : cells_) {
        std::fill(row.begin(), row.end(), empty_cell_);
    }
}

void FrameBuffer::clear_with_color(uint32_t bg) {
    Cell cell = empty_cell_;
    cell.bg = bg;
    for (auto& row : cells_) {
        std::fill(row.begin(), row.end(), cell);
    }
}

void FrameBuffer::set_cell(int x, int y, const Cell& cell) {
    if (x >= 0 && x < width_ && y >= 0 && y < height_) {
        cells_[y][x] = cell;
    }
}

const Cell& FrameBuffer::get_cell(int x, int y) const {
    if (x >= 0 && x < width_ && y >= 0 && y < height_) {
        return cells_[y][x];
    }
    return empty_cell_;
}

void FrameBuffer::set_text(int x, int y, const std::string& text,
                           uint32_t fg, uint32_t bg, uint8_t attrs) {
    if (y < 0 || y >= height_) return;

    size_t i = 0;
    int cur_x = x;

    while (i < text.length() && cur_x < width_) {
        if (cur_x < 0) {
            // Skip characters before visible area
            i += Unicode::char_byte_length(text, i);
            cur_x++;
            continue;
        }

        size_t byte_len = Unicode::char_byte_length(text, i);
        std::string ch = text.substr(i, byte_len);

        // Determine character width
        size_t char_width = 1;
        unsigned char c = text[i];
        if ((c & 0xF0) == 0xE0 || (c & 0xF8) == 0xF0) {
            char_width = 2; // CJK or emoji
        }

        Cell cell;
        cell.content = ch;
        cell.fg = fg;
        cell.bg = bg;
        cell.attrs = attrs;

        set_cell(cur_x, y, cell);

        // For wide characters, mark next cell as placeholder
        if (char_width == 2 && cur_x + 1 < width_) {
            Cell placeholder;
            placeholder.content = "";  // Empty = continuation of previous cell
            placeholder.fg = fg;
            placeholder.bg = bg;
            placeholder.attrs = attrs;
            set_cell(cur_x + 1, y, placeholder);
        }

        cur_x += char_width;
        i += byte_len;
    }
}

// ============================================================================
// Renderer Implementation
// ============================================================================

Renderer::Renderer(Terminal& terminal)
    : terminal_(terminal), prev_buffer_(1, 1) {
    int w, h;
    terminal_.get_size(w, h);
    prev_buffer_.resize(w, h);
}

void Renderer::render(const FrameBuffer& buffer) {
    int w = buffer.width();
    int h = buffer.height();

    // Check if resize needed
    if (prev_buffer_.width() != w || prev_buffer_.height() != h) {
        prev_buffer_.resize(w, h);
        need_full_redraw_ = true;
    }

    terminal_.hide_cursor();

    uint32_t last_fg = 0xFFFFFFFF;  // Invalid color to force first set
    uint32_t last_bg = 0xFFFFFFFF;
    uint8_t last_attrs = 0xFF;
    int last_x = -2;

    // 批量输出缓冲
    std::string batch_text;
    int batch_start_x = 0;
    int batch_y = 0;
    uint32_t batch_fg = 0;
    uint32_t batch_bg = 0;
    uint8_t batch_attrs = 0;

    auto flush_batch = [&]() {
        if (batch_text.empty()) return;

        terminal_.move_cursor(batch_start_x, batch_y);

        if (batch_fg != last_fg) {
            terminal_.set_foreground(batch_fg);
            last_fg = batch_fg;
        }
        if (batch_bg != last_bg) {
            terminal_.set_background(batch_bg);
            last_bg = batch_bg;
        }
        if (batch_attrs != last_attrs) {
            terminal_.reset_attributes();
            if (batch_attrs & ATTR_BOLD) terminal_.set_bold(true);
            if (batch_attrs & ATTR_ITALIC) terminal_.set_italic(true);
            if (batch_attrs & ATTR_UNDERLINE) terminal_.set_underline(true);
            if (batch_attrs & ATTR_REVERSE) terminal_.set_reverse(true);
            if (batch_attrs & ATTR_DIM) terminal_.set_dim(true);
            last_attrs = batch_attrs;
            terminal_.set_foreground(batch_fg);
            terminal_.set_background(batch_bg);
        }

        terminal_.print(batch_text);
        batch_text.clear();
    };

    for (int y = 0; y < h; y++) {
        for (int x = 0; x < w; x++) {
            const Cell& cell = buffer.get_cell(x, y);
            const Cell& prev = prev_buffer_.get_cell(x, y);

            // Skip if unchanged and not forcing redraw
            if (!need_full_redraw_ && cell == prev) {
                flush_batch();
                last_x = -2;
                continue;
            }

            // Skip placeholder cells (continuation of wide chars)
            if (cell.content.empty()) {
                continue;
            }

            // 检查是否可以添加到批量输出
            bool can_batch = (y == batch_y) &&
                            (x == last_x + 1 || batch_text.empty()) &&
                            (cell.fg == batch_fg || batch_text.empty()) &&
                            (cell.bg == batch_bg || batch_text.empty()) &&
                            (cell.attrs == batch_attrs || batch_text.empty());

            if (!can_batch) {
                flush_batch();
                batch_start_x = x;
                batch_y = y;
                batch_fg = cell.fg;
                batch_bg = cell.bg;
                batch_attrs = cell.attrs;
            }

            batch_text += cell.content;
            last_x = x;
        }

        // 行末刷新
        flush_batch();
        last_x = -2;
    }

    flush_batch();

    terminal_.reset_colors();
    terminal_.reset_attributes();
    terminal_.refresh();

    // Copy current buffer to previous for next diff
    for (int y = 0; y < h; y++) {
        for (int x = 0; x < w; x++) {
            const_cast<FrameBuffer&>(prev_buffer_).set_cell(x, y, buffer.get_cell(x, y));
        }
    }

    need_full_redraw_ = false;
}

void Renderer::force_redraw() {
    need_full_redraw_ = true;
}

} // namespace tut
