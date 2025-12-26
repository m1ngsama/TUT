/**
 * test_layout.cpp - Layout引擎测试
 *
 * 测试内容：
 * 1. DOM树构建
 * 2. 布局计算
 * 3. 文档渲染演示
 */

#include "render/terminal.h"
#include "render/renderer.h"
#include "render/layout.h"
#include "render/colors.h"
#include "dom_tree.h"
#include <iostream>
#include <string>
#include <ncurses.h>

using namespace tut;

void test_image_placeholder() {
    std::cout << "=== 图片占位符测试 ===\n";

    std::string html = R"(
<!DOCTYPE html>
<html>
<head><title>图片测试</title></head>
<body>
    <h1>图片测试页面</h1>
    <p>下面是一些图片:</p>
    <img src="https://example.com/photo.png" alt="Example Photo" />
    <p>中间文本</p>
    <img src="logo.jpg" />
    <img alt="Only alt text" />
    <img />
</body>
</html>
)";

    DomTreeBuilder builder;
    DocumentTree doc = builder.build(html, "test://");

    LayoutEngine engine(80);
    LayoutResult layout = engine.layout(doc);

    std::cout << "图片测试 - 总块数: " << layout.blocks.size() << "\n";
    std::cout << "图片测试 - 总行数: " << layout.total_lines << "\n";

    // 检查渲染输出
    int img_count = 0;
    for (const auto& block : layout.blocks) {
        if (block.type == ElementType::IMAGE) {
            img_count++;
            if (!block.lines.empty() && !block.lines[0].spans.empty()) {
                std::cout << "  图片 " << img_count << ": " << block.lines[0].spans[0].text << "\n";
            }
        }
    }
    std::cout << "找到 " << img_count << " 个图片块\n\n";
}

void test_layout_basic() {
    std::cout << "=== Layout 基础测试 ===\n";

    // 测试HTML
    std::string html = R"(
<!DOCTYPE html>
<html>
<head><title>测试页面</title></head>
<body>
    <h1>TUT 2.0 布局引擎测试</h1>
    <p>这是一个段落，用于测试文本换行功能。当文本超过视口宽度时，应该自动换行到下一行。</p>
    <h2>列表测试</h2>
    <ul>
        <li>无序列表项目 1</li>
        <li>无序列表项目 2</li>
        <li>无序列表项目 3</li>
    </ul>
    <h2>链接测试</h2>
    <p>这是一个 <a href="https://example.com">链接示例</a>，点击可以访问。</p>
    <blockquote>这是一段引用文本，应该带有左边框标记。</blockquote>
    <hr>
    <p>页面结束。</p>
</body>
</html>
)";

    // 构建DOM树
    DomTreeBuilder builder;
    DocumentTree doc = builder.build(html, "test://");
    std::cout << "DOM树构建: OK\n";
    std::cout << "标题: " << doc.title << "\n";
    std::cout << "链接数: " << doc.links.size() << "\n";

    // 布局计算
    LayoutEngine engine(80);
    LayoutResult layout = engine.layout(doc);
    std::cout << "布局计算: OK\n";
    std::cout << "布局块数: " << layout.blocks.size() << "\n";
    std::cout << "总行数: " << layout.total_lines << "\n";

    // 打印布局块信息
    std::cout << "\n布局块详情:\n";
    int block_num = 0;
    for (const auto& block : layout.blocks) {
        std::cout << "  Block " << block_num++ << ": "
                  << block.lines.size() << " lines, "
                  << "margin_top=" << block.margin_top << ", "
                  << "margin_bottom=" << block.margin_bottom << "\n";
    }

    std::cout << "\nLayout 基础测试完成!\n";
}

void demo_layout_render(Terminal& term) {
    int w, h;
    term.get_size(w, h);

    // 创建测试HTML
    std::string html = R"(
<!DOCTYPE html>
<html>
<head><title>TUT 2.0 布局演示</title></head>
<body>
    <h1>TUT 2.0 - 终端浏览器</h1>

    <p>这是一个现代化的终端浏览器，支持 True Color 渲染、Unicode 字符以及差分渲染优化。</p>

    <h2>主要特性</h2>
    <ul>
        <li>True Color 24位色彩支持</li>
        <li>Unicode 字符正确显示（包括CJK字符）</li>
        <li>差分渲染提升性能</li>
        <li>温暖护眼的配色方案</li>
    </ul>

    <h2>链接示例</h2>
    <p>访问 <a href="https://example.com">Example</a> 或 <a href="https://github.com">GitHub</a> 了解更多信息。</p>

    <h3>引用块</h3>
    <blockquote>Unix哲学：做一件事，把它做好。</blockquote>

    <hr>

    <p>使用 j/k 滚动，q 退出。</p>
</body>
</html>
)";

    // 构建DOM树
    DomTreeBuilder builder;
    DocumentTree doc = builder.build(html, "demo://");

    // 布局计算
    LayoutEngine engine(w);
    LayoutResult layout = engine.layout(doc);

    // 创建帧缓冲区和渲染器
    FrameBuffer fb(w, h - 2);  // 留出状态栏空间
    Renderer renderer(term);
    DocumentRenderer doc_renderer(fb);

    int scroll_offset = 0;
    int max_scroll = std::max(0, layout.total_lines - (h - 2));
    int active_link = -1;
    int num_links = static_cast<int>(doc.links.size());

    bool running = true;
    while (running) {
        // 清空缓冲区
        fb.clear_with_color(colors::BG_PRIMARY);

        // 渲染文档
        RenderContext render_ctx;
        render_ctx.active_link = active_link;
        doc_renderer.render(layout, scroll_offset, render_ctx);

        // 渲染状态栏
        std::string status = layout.title + " | 行 " + std::to_string(scroll_offset + 1) +
                           "/" + std::to_string(layout.total_lines);
        if (active_link >= 0 && active_link < num_links) {
            status += " | 链接: " + doc.links[active_link].url;
        }
        // 截断过长的状态栏
        if (Unicode::display_width(status) > static_cast<size_t>(w - 2)) {
            status = status.substr(0, w - 5) + "...";
        }

        // 状态栏在最后一行
        for (int x = 0; x < w; ++x) {
            fb.set_cell(x, h - 2, Cell{" ", colors::STATUSBAR_FG, colors::STATUSBAR_BG, ATTR_NONE});
        }
        fb.set_text(1, h - 2, status, colors::STATUSBAR_FG, colors::STATUSBAR_BG);

        // 渲染到终端
        renderer.render(fb);

        // 处理输入
        int key = term.get_key(100);
        switch (key) {
            case 'q':
            case 'Q':
                running = false;
                break;
            case 'j':
            case KEY_DOWN:
                if (scroll_offset < max_scroll) scroll_offset++;
                break;
            case 'k':
            case KEY_UP:
                if (scroll_offset > 0) scroll_offset--;
                break;
            case ' ':
            case KEY_NPAGE:
                scroll_offset = std::min(scroll_offset + (h - 3), max_scroll);
                break;
            case 'b':
            case KEY_PPAGE:
                scroll_offset = std::max(scroll_offset - (h - 3), 0);
                break;
            case 'g':
            case KEY_HOME:
                scroll_offset = 0;
                break;
            case 'G':
            case KEY_END:
                scroll_offset = max_scroll;
                break;
            case '\t':  // Tab键切换链接
                if (num_links > 0) {
                    active_link = (active_link + 1) % num_links;
                }
                break;
            case KEY_BTAB:  // Shift+Tab
                if (num_links > 0) {
                    active_link = (active_link - 1 + num_links) % num_links;
                }
                break;
        }
    }
}

int main() {
    // 先运行非终端测试
    test_image_placeholder();
    test_layout_basic();

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

    demo_layout_render(term);

    term.show_cursor();
    term.use_alternate_screen(false);
    term.cleanup();

    std::cout << "Layout 测试完成!\n";
    return 0;
}
