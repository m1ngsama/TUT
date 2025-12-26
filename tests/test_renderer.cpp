/**
 * test_renderer.cpp - FrameBuffer 和 Renderer 测试
 *
 * 测试内容：
 * 1. Unicode字符宽度计算
 * 2. FrameBuffer操作
 * 3. 差分渲染演示
 */

#include "render/terminal.h"
#include "render/renderer.h"
#include "render/colors.h"
#include "render/decorations.h"
#include "utils/unicode.h"
#include <iostream>
#include <thread>
#include <chrono>

using namespace tut;

void test_unicode() {
    std::cout << "=== Unicode 测试 ===\n";

    // 测试用例
    struct TestCase {
        std::string text;
        size_t expected_width;
        const char* description;
    };

    TestCase tests[] = {
        {"Hello", 5, "ASCII"},
        {"你好", 4, "中文(2字符,宽度4)"},
        {"Hello世界", 9, "混合ASCII+中文"},
        {"🎉", 2, "Emoji"},
        {"café", 4, "带重音符号"},
    };

    bool all_passed = true;
    for (const auto& tc : tests) {
        size_t width = Unicode::display_width(tc.text);
        bool pass = (width == tc.expected_width);
        std::cout << (pass ? "[OK] " : "[FAIL] ")
                  << tc.description << ": \"" << tc.text << "\" "
                  << "width=" << width
                  << " (expected " << tc.expected_width << ")\n";
        if (!pass) all_passed = false;
    }

    std::cout << (all_passed ? "\n所有Unicode测试通过!\n" : "\n部分测试失败!\n");
}

void test_framebuffer() {
    std::cout << "\n=== FrameBuffer 测试 ===\n";

    FrameBuffer fb(80, 24);
    std::cout << "创建 80x24 FrameBuffer: OK\n";

    // 测试set_text
    fb.set_text(0, 0, "Hello World", colors::FG_PRIMARY, colors::BG_PRIMARY);
    std::cout << "set_text ASCII: OK\n";

    fb.set_text(0, 1, "你好世界", colors::H1_FG, colors::BG_PRIMARY);
    std::cout << "set_text 中文: OK\n";

    // 验证单元格
    const Cell& cell = fb.get_cell(0, 0);
    if (cell.content == "H" && cell.fg == colors::FG_PRIMARY) {
        std::cout << "get_cell 验证: OK\n";
    } else {
        std::cout << "get_cell 验证: FAIL\n";
    }

    std::cout << "FrameBuffer 测试完成!\n";
}

void demo_renderer(Terminal& term) {
    int w, h;
    term.get_size(w, h);

    FrameBuffer fb(w, h);
    Renderer renderer(term);

    // 清屏并显示标题
    fb.clear_with_color(colors::BG_PRIMARY);

    // 标题
    std::string title = "TUT 2.0 - Renderer Demo";
    int title_x = (w - Unicode::display_width(title)) / 2;
    fb.set_text(title_x, 1, title, colors::H1_FG, colors::BG_PRIMARY, ATTR_BOLD);

    // 分隔线
    std::string line = make_horizontal_line(w - 4, chars::SGL_HORIZONTAL);
    fb.set_text(2, 2, line, colors::BORDER, colors::BG_PRIMARY);

    // 颜色示例
    fb.set_text(2, 4, "颜色示例:", colors::FG_PRIMARY, colors::BG_PRIMARY, ATTR_BOLD);
    fb.set_text(4, 5, chars::BULLET + std::string(" H1标题色"), colors::H1_FG, colors::BG_PRIMARY);
    fb.set_text(4, 6, chars::BULLET + std::string(" H2标题色"), colors::H2_FG, colors::BG_PRIMARY);
    fb.set_text(4, 7, chars::BULLET + std::string(" H3标题色"), colors::H3_FG, colors::BG_PRIMARY);
    fb.set_text(4, 8, chars::BULLET + std::string(" 链接色"), colors::LINK_FG, colors::BG_PRIMARY);

    // 装饰字符示例
    fb.set_text(2, 10, "装饰字符:", colors::FG_PRIMARY, colors::BG_PRIMARY, ATTR_BOLD);
    fb.set_text(4, 11, std::string(chars::DBL_TOP_LEFT) + make_horizontal_line(20, chars::DBL_HORIZONTAL) + chars::DBL_TOP_RIGHT,
                colors::BORDER, colors::BG_PRIMARY);
    fb.set_text(4, 12, std::string(chars::DBL_VERTICAL) + "  双线边框示例    " + chars::DBL_VERTICAL,
                colors::BORDER, colors::BG_PRIMARY);
    fb.set_text(4, 13, std::string(chars::DBL_BOTTOM_LEFT) + make_horizontal_line(20, chars::DBL_HORIZONTAL) + chars::DBL_BOTTOM_RIGHT,
                colors::BORDER, colors::BG_PRIMARY);

    // Unicode宽度示例
    fb.set_text(2, 15, "Unicode宽度:", colors::FG_PRIMARY, colors::BG_PRIMARY, ATTR_BOLD);
    fb.set_text(4, 16, "ASCII: Hello (5)", colors::FG_SECONDARY, colors::BG_PRIMARY);
    fb.set_text(4, 17, "中文: 你好世界 (8)", colors::FG_SECONDARY, colors::BG_PRIMARY);

    // 提示
    fb.set_text(2, h - 2, "按 'q' 退出", colors::FG_DIM, colors::BG_PRIMARY);

    // 渲染
    renderer.render(fb);

    // 等待退出
    while (true) {
        int key = term.get_key(100);
        if (key == 'q' || key == 'Q') break;
    }
}

int main() {
    // 先运行非终端测试
    test_unicode();
    test_framebuffer();

    std::cout << "\n按回车键进入交互演示 (或 Ctrl+C 退出)...\n";
    std::cin.get();

    // 交互演示
    Terminal term;
    if (!term.init()) {
        std::cerr << "终端初始化失败!\n";
        return 1;
    }

    term.use_alternate_screen(true);
    term.hide_cursor();

    demo_renderer(term);

    term.show_cursor();
    term.use_alternate_screen(false);
    term.cleanup();

    std::cout << "Renderer 测试完成!\n";
    return 0;
}
