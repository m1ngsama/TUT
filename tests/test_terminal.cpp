/**
 * test_terminal.cpp - Terminal类True Color功能测试
 *
 * 测试内容:
 * 1. True Color (24-bit RGB) 支持
 * 2. 文本属性 (粗体、斜体、下划线)
 * 3. Unicode字符显示
 * 4. 终端能力检测
 */

#include "terminal.h"
#include <iostream>
#include <thread>
#include <chrono>

using namespace tut;

void test_true_color(Terminal& term) {
    term.clear();

    // 标题
    term.move_cursor(0, 0);
    term.set_bold(true);
    term.set_foreground(0xE8C48C);  // 暖金色
    term.print("TUT 2.0 - True Color Test");
    term.reset_attributes();

    // 能力检测报告
    int y = 2;
    term.move_cursor(0, y++);
    term.print("Terminal Capabilities:");

    term.move_cursor(0, y++);
    term.print("  True Color: ");
    if (term.supports_true_color()) {
        term.set_foreground(0x00FF00);
        term.print("✓ Supported");
    } else {
        term.set_foreground(0xFF0000);
        term.print("✗ Not Supported");
    }
    term.reset_colors();

    term.move_cursor(0, y++);
    term.print("  Mouse:      ");
    if (term.supports_mouse()) {
        term.set_foreground(0x00FF00);
        term.print("✓ Supported");
    } else {
        term.set_foreground(0xFF0000);
        term.print("✗ Not Supported");
    }
    term.reset_colors();

    term.move_cursor(0, y++);
    term.print("  Unicode:    ");
    if (term.supports_unicode()) {
        term.set_foreground(0x00FF00);
        term.print("✓ Supported");
    } else {
        term.set_foreground(0xFF0000);
        term.print("✗ Not Supported");
    }
    term.reset_colors();

    term.move_cursor(0, y++);
    term.print("  Italic:     ");
    if (term.supports_italic()) {
        term.set_foreground(0x00FF00);
        term.print("✓ Supported");
    } else {
        term.set_foreground(0xFF0000);
        term.print("✗ Not Supported");
    }
    term.reset_colors();

    y++;

    // 报纸风格颜色主题测试
    term.move_cursor(0, y++);
    term.set_bold(true);
    term.print("Newspaper Color Theme:");
    term.reset_attributes();

    y++;

    // H1 颜色
    term.move_cursor(0, y++);
    term.set_bold(true);
    term.set_foreground(0xE8C48C);  // 暖金色
    term.print("  H1 Heading - Warm Gold (0xE8C48C)");
    term.reset_attributes();

    // H2 颜色
    term.move_cursor(0, y++);
    term.set_bold(true);
    term.set_foreground(0xD4B078);  // 较暗金色
    term.print("  H2 Heading - Dark Gold (0xD4B078)");
    term.reset_attributes();

    // H3 颜色
    term.move_cursor(0, y++);
    term.set_bold(true);
    term.set_foreground(0xC09C64);  // 青铜色
    term.print("  H3 Heading - Bronze (0xC09C64)");
    term.reset_attributes();

    y++;

    // 链接颜色
    term.move_cursor(0, y++);
    term.set_foreground(0x87AFAF);  // 青色
    term.set_underline(true);
    term.print("  Link - Teal (0x87AFAF)");
    term.reset_attributes();

    // 悬停链接
    term.move_cursor(0, y++);
    term.set_foreground(0xA7CFCF);  // 浅青色
    term.set_underline(true);
    term.print("  Link Hover - Light Teal (0xA7CFCF)");
    term.reset_attributes();

    y++;

    // 正文颜色
    term.move_cursor(0, y++);
    term.set_foreground(0xD0D0D0);  // 浅灰
    term.print("  Body Text - Light Gray (0xD0D0D0)");
    term.reset_colors();

    // 次要文本
    term.move_cursor(0, y++);
    term.set_foreground(0x909090);  // 中灰
    term.print("  Secondary Text - Medium Gray (0x909090)");
    term.reset_colors();

    y++;

    // Unicode装饰测试
    term.move_cursor(0, y++);
    term.set_bold(true);
    term.print("Unicode Box Drawing:");
    term.reset_attributes();

    y++;

    // 双线框
    term.move_cursor(0, y++);
    term.set_foreground(0x404040);
    term.print("  ╔═══════════════════════════════════╗");
    term.move_cursor(0, y++);
    term.print("  ║  Double Border for H1 Headings    ║");
    term.move_cursor(0, y++);
    term.print("  ╚═══════════════════════════════════╝");
    term.reset_colors();

    y++;

    // 单线框
    term.move_cursor(0, y++);
    term.set_foreground(0x404040);
    term.print("  ┌───────────────────────────────────┐");
    term.move_cursor(0, y++);
    term.print("  │  Single Border for Code Blocks    │");
    term.move_cursor(0, y++);
    term.print("  └───────────────────────────────────┘");
    term.reset_colors();

    y++;

    // 引用块
    term.move_cursor(0, y++);
    term.set_foreground(0x6A8F8F);
    term.print("  ┃ Blockquote with heavy vertical bar");
    term.reset_colors();

    y++;

    // 列表符号
    term.move_cursor(0, y++);
    term.print("  • Bullet point (level 1)");
    term.move_cursor(0, y++);
    term.print("    ◦ Circle (level 2)");
    term.move_cursor(0, y++);
    term.print("      ▪ Square (level 3)");

    y += 2;

    // 提示
    term.move_cursor(0, y++);
    term.set_dim(true);
    term.print("Press any key to exit...");
    term.reset_attributes();

    term.refresh();
}

int main() {
    Terminal term;

    if (!term.init()) {
        std::cerr << "Failed to initialize terminal" << std::endl;
        return 1;
    }

    try {
        test_true_color(term);

        // 等待按键
        term.get_key(-1);

        term.cleanup();

    } catch (const std::exception& e) {
        term.cleanup();
        std::cerr << "Error: " << e.what() << std::endl;
        return 1;
    }

    return 0;
}
