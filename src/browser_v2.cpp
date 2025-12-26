#include "browser_v2.h"
#include "dom_tree.h"
#include "render/colors.h"
#include "render/decorations.h"
#include "utils/unicode.h"
#include <algorithm>
#include <sstream>
#include <map>
#include <cctype>
#include <cstdio>
#include <chrono>
#include <ncurses.h>

using namespace tut;

// 缓存条目
struct CacheEntry {
    DocumentTree tree;
    std::string html;
    std::chrono::steady_clock::time_point timestamp;

    bool is_expired(int max_age_seconds = 300) const {
        auto now = std::chrono::steady_clock::now();
        auto age = std::chrono::duration_cast<std::chrono::seconds>(now - timestamp).count();
        return age > max_age_seconds;
    }
};

class BrowserV2::Impl {
public:
    // 网络和解析
    HttpClient http_client;
    HtmlParser html_parser;
    InputHandler input_handler;

    // 新渲染系统
    Terminal terminal;
    std::unique_ptr<FrameBuffer> framebuffer;
    std::unique_ptr<Renderer> renderer;
    std::unique_ptr<LayoutEngine> layout_engine;

    // 文档状态
    DocumentTree current_tree;
    LayoutResult current_layout;
    std::string current_url;
    std::vector<std::string> history;
    int history_pos = -1;

    // 视图状态
    int scroll_pos = 0;
    int active_link = -1;
    int active_field = -1;
    std::string status_message;
    std::string search_term;

    int screen_width = 0;
    int screen_height = 0;

    // Marks support
    std::map<char, int> marks;

    // 搜索相关
    SearchContext search_ctx;

    // 页面缓存
    std::map<std::string, CacheEntry> page_cache;
    static constexpr int CACHE_MAX_AGE = 300;  // 5分钟缓存
    static constexpr size_t CACHE_MAX_SIZE = 20;  // 最多缓存20个页面

    bool init_screen() {
        if (!terminal.init()) {
            return false;
        }

        terminal.get_size(screen_width, screen_height);
        terminal.use_alternate_screen(true);
        terminal.hide_cursor();

        // 创建渲染组件
        framebuffer = std::make_unique<FrameBuffer>(screen_width, screen_height);
        renderer = std::make_unique<Renderer>(terminal);
        layout_engine = std::make_unique<LayoutEngine>(screen_width);

        return true;
    }

    void cleanup_screen() {
        terminal.show_cursor();
        terminal.use_alternate_screen(false);
        terminal.cleanup();
    }

    void handle_resize() {
        terminal.get_size(screen_width, screen_height);
        framebuffer = std::make_unique<FrameBuffer>(screen_width, screen_height);
        layout_engine->set_viewport_width(screen_width);

        // 重新布局当前文档
        if (current_tree.root) {
            current_layout = layout_engine->layout(current_tree);
        }

        renderer->force_redraw();
    }

    bool load_page(const std::string& url, bool force_refresh = false) {
        // 检查缓存
        auto cache_it = page_cache.find(url);
        bool use_cache = !force_refresh && cache_it != page_cache.end() &&
                        !cache_it->second.is_expired(CACHE_MAX_AGE);

        if (use_cache) {
            status_message = "⚡ Loading from cache...";
            draw_screen();

            // 使用缓存的文档树
            // 注意：需要重新解析因为DocumentTree包含unique_ptr
            current_tree = html_parser.parse_tree(cache_it->second.html, url);
            status_message = "⚡ " + (current_tree.title.empty() ? url : current_tree.title);
        } else {
            status_message = "⏳ Connecting to " + extract_host(url) + "...";
            draw_screen();

            auto response = http_client.fetch(url);

            if (!response.is_success()) {
                status_message = "❌ " + (response.error_message.empty() ?
                    "HTTP " + std::to_string(response.status_code) :
                    response.error_message);
                return false;
            }

            status_message = "📄 Parsing HTML...";
            draw_screen();

            // 解析HTML
            current_tree = html_parser.parse_tree(response.body, url);

            // 添加到缓存
            add_to_cache(url, response.body);

            status_message = current_tree.title.empty() ? url : current_tree.title;
        }

        // 布局计算
        current_layout = layout_engine->layout(current_tree);

        current_url = url;
        scroll_pos = 0;
        active_link = current_tree.links.empty() ? -1 : 0;
        active_field = current_tree.form_fields.empty() ? -1 : 0;
        search_ctx = SearchContext();  // 清除搜索状态
        search_term.clear();

        // 更新历史（仅在非刷新时）
        if (!force_refresh) {
            if (history_pos >= 0 && history_pos < static_cast<int>(history.size()) - 1) {
                history.erase(history.begin() + history_pos + 1, history.end());
            }
            history.push_back(url);
            history_pos = history.size() - 1;
        }

        return true;
    }

    void add_to_cache(const std::string& url, const std::string& html) {
        // 限制缓存大小
        if (page_cache.size() >= CACHE_MAX_SIZE) {
            // 移除最老的缓存条目
            auto oldest = page_cache.begin();
            for (auto it = page_cache.begin(); it != page_cache.end(); ++it) {
                if (it->second.timestamp < oldest->second.timestamp) {
                    oldest = it;
                }
            }
            page_cache.erase(oldest);
        }

        CacheEntry entry;
        entry.html = html;
        entry.timestamp = std::chrono::steady_clock::now();
        page_cache[url] = std::move(entry);
    }

    // 从URL中提取主机名
    std::string extract_host(const std::string& url) {
        // 简单提取：找到://之后的部分，到第一个/为止
        size_t proto_end = url.find("://");
        if (proto_end == std::string::npos) {
            return url;
        }
        size_t host_start = proto_end + 3;
        size_t host_end = url.find('/', host_start);
        if (host_end == std::string::npos) {
            return url.substr(host_start);
        }
        return url.substr(host_start, host_end - host_start);
    }

    void draw_screen() {
        // 清空缓冲区
        framebuffer->clear_with_color(colors::BG_PRIMARY);

        int content_height = screen_height - 1;  // 留出状态栏

        // 渲染文档内容
        RenderContext render_ctx;
        render_ctx.active_link = active_link;
        render_ctx.active_field = active_field;
        render_ctx.search = search_ctx.enabled ? &search_ctx : nullptr;

        DocumentRenderer doc_renderer(*framebuffer);
        doc_renderer.render(current_layout, scroll_pos, render_ctx);

        // 渲染状态栏
        draw_status_bar(content_height);

        // 渲染到终端
        renderer->render(*framebuffer);
    }

    void draw_status_bar(int y) {
        // 状态栏背景
        for (int x = 0; x < screen_width; ++x) {
            framebuffer->set_cell(x, y, Cell{" ", colors::STATUSBAR_FG, colors::STATUSBAR_BG, ATTR_NONE});
        }

        // 左侧: 模式
        std::string mode_str;
        InputMode mode = input_handler.get_mode();
        switch (mode) {
            case InputMode::NORMAL: mode_str = "NORMAL"; break;
            case InputMode::COMMAND:
            case InputMode::SEARCH: mode_str = input_handler.get_buffer(); break;
            default: mode_str = ""; break;
        }
        framebuffer->set_text(1, y, mode_str, colors::STATUSBAR_FG, colors::STATUSBAR_BG);

        // 中间: 状态消息或链接URL
        std::string display_msg;
        if (mode == InputMode::NORMAL) {
            if (active_link >= 0 && active_link < static_cast<int>(current_tree.links.size())) {
                display_msg = current_tree.links[active_link].url;
            }
            if (display_msg.empty()) {
                display_msg = status_message;
            }

            if (!display_msg.empty()) {
                // 截断过长的消息
                size_t max_len = screen_width - mode_str.length() - 20;
                if (display_msg.length() > max_len) {
                    display_msg = display_msg.substr(0, max_len - 3) + "...";
                }
                int msg_x = static_cast<int>(mode_str.length()) + 3;
                framebuffer->set_text(msg_x, y, display_msg, colors::STATUSBAR_FG, colors::STATUSBAR_BG);
            }
        }

        // 右侧: 位置信息
        int total_lines = current_layout.total_lines;
        int visible_lines = screen_height - 1;
        int percentage = (total_lines > 0 && scroll_pos + visible_lines < total_lines) ?
                         (scroll_pos * 100) / total_lines : 100;
        if (total_lines == 0) percentage = 0;

        std::string pos_str = std::to_string(scroll_pos + 1) + "/" +
                             std::to_string(total_lines) + " " +
                             std::to_string(percentage) + "%";
        int pos_x = screen_width - static_cast<int>(pos_str.length()) - 1;
        framebuffer->set_text(pos_x, y, pos_str, colors::STATUSBAR_FG, colors::STATUSBAR_BG);
    }

    void handle_action(const InputResult& result) {
        int visible_lines = screen_height - 1;
        int max_scroll = std::max(0, current_layout.total_lines - visible_lines);
        int count = result.has_count ? result.count : 1;

        switch (result.action) {
            case Action::SCROLL_UP:
                scroll_pos = std::max(0, scroll_pos - count);
                break;
            case Action::SCROLL_DOWN:
                scroll_pos = std::min(max_scroll, scroll_pos + count);
                break;
            case Action::SCROLL_PAGE_UP:
                scroll_pos = std::max(0, scroll_pos - visible_lines);
                break;
            case Action::SCROLL_PAGE_DOWN:
                scroll_pos = std::min(max_scroll, scroll_pos + visible_lines);
                break;
            case Action::GOTO_TOP:
                scroll_pos = 0;
                break;
            case Action::GOTO_BOTTOM:
                scroll_pos = max_scroll;
                break;
            case Action::GOTO_LINE:
                if (result.number > 0) {
                    scroll_pos = std::min(result.number - 1, max_scroll);
                }
                break;

            case Action::NEXT_LINK:
                if (!current_tree.links.empty()) {
                    active_link = (active_link + 1) % current_tree.links.size();
                    scroll_to_link(active_link);
                }
                break;

            case Action::PREV_LINK:
                if (!current_tree.links.empty()) {
                    active_link = (active_link - 1 + current_tree.links.size()) % current_tree.links.size();
                    scroll_to_link(active_link);
                }
                break;

            case Action::FOLLOW_LINK:
                if (active_link >= 0 && active_link < static_cast<int>(current_tree.links.size())) {
                    load_page(current_tree.links[active_link].url);
                }
                break;

            case Action::GO_BACK:
                if (history_pos > 0) {
                    history_pos--;
                    load_page(history[history_pos]);
                }
                break;

            case Action::GO_FORWARD:
                if (history_pos < static_cast<int>(history.size()) - 1) {
                    history_pos++;
                    load_page(history[history_pos]);
                }
                break;

            case Action::OPEN_URL:
                if (!result.text.empty()) {
                    load_page(result.text);
                }
                break;

            case Action::REFRESH:
                if (!current_url.empty()) {
                    load_page(current_url, true);  // 强制刷新，跳过缓存
                }
                break;

            case Action::SEARCH_FORWARD: {
                int count = perform_search(result.text);
                if (count > 0) {
                    status_message = "Match 1/" + std::to_string(count);
                } else if (!result.text.empty()) {
                    status_message = "Pattern not found: " + result.text;
                }
                break;
            }

            case Action::SEARCH_NEXT:
                search_next();
                break;

            case Action::SEARCH_PREV:
                search_prev();
                break;

            case Action::HELP:
                show_help();
                break;

            case Action::QUIT:
                break; // 在main loop处理

            default:
                break;
        }
    }

    // 执行搜索，返回匹配数量
    int perform_search(const std::string& term) {
        search_ctx.matches.clear();
        search_ctx.current_match_idx = -1;
        search_ctx.enabled = false;

        if (term.empty()) {
            return 0;
        }

        search_term = term;
        search_ctx.enabled = true;

        // 遍历所有布局块和行，查找匹配
        int doc_line = 0;
        for (const auto& block : current_layout.blocks) {
            // 上边距
            doc_line += block.margin_top;

            // 内容行
            for (const auto& line : block.lines) {
                // 构建整行文本用于搜索
                std::string line_text;

                for (const auto& span : line.spans) {
                    line_text += span.text;
                }

                // 搜索匹配（大小写不敏感）
                std::string lower_line = line_text;
                std::string lower_term = term;
                std::transform(lower_line.begin(), lower_line.end(), lower_line.begin(), ::tolower);
                std::transform(lower_term.begin(), lower_term.end(), lower_term.begin(), ::tolower);

                size_t pos = 0;
                while ((pos = lower_line.find(lower_term, pos)) != std::string::npos) {
                    SearchMatch match;
                    match.line = doc_line;
                    match.start_col = line.indent + static_cast<int>(pos);
                    match.length = static_cast<int>(term.length());
                    search_ctx.matches.push_back(match);
                    pos += 1;  // 继续搜索下一个匹配
                }

                doc_line++;
            }

            // 下边距
            doc_line += block.margin_bottom;
        }

        // 如果有匹配，跳转到第一个
        if (!search_ctx.matches.empty()) {
            search_ctx.current_match_idx = 0;
            scroll_to_match(0);
        }

        return static_cast<int>(search_ctx.matches.size());
    }

    // 跳转到指定匹配
    void scroll_to_match(int idx) {
        if (idx < 0 || idx >= static_cast<int>(search_ctx.matches.size())) {
            return;
        }

        search_ctx.current_match_idx = idx;
        int match_line = search_ctx.matches[idx].line;
        int visible_lines = screen_height - 1;

        // 确保匹配行在可见区域
        if (match_line < scroll_pos) {
            scroll_pos = match_line;
        } else if (match_line >= scroll_pos + visible_lines) {
            scroll_pos = match_line - visible_lines / 2;
        }

        int max_scroll = std::max(0, current_layout.total_lines - visible_lines);
        scroll_pos = std::max(0, std::min(scroll_pos, max_scroll));
    }

    // 搜索下一个
    void search_next() {
        if (search_ctx.matches.empty()) {
            if (!search_term.empty()) {
                status_message = "Pattern not found: " + search_term;
            }
            return;
        }

        search_ctx.current_match_idx = (search_ctx.current_match_idx + 1) % search_ctx.matches.size();
        scroll_to_match(search_ctx.current_match_idx);
        status_message = "Match " + std::to_string(search_ctx.current_match_idx + 1) +
                        "/" + std::to_string(search_ctx.matches.size());
    }

    // 搜索上一个
    void search_prev() {
        if (search_ctx.matches.empty()) {
            if (!search_term.empty()) {
                status_message = "Pattern not found: " + search_term;
            }
            return;
        }

        search_ctx.current_match_idx = (search_ctx.current_match_idx - 1 + search_ctx.matches.size()) % search_ctx.matches.size();
        scroll_to_match(search_ctx.current_match_idx);
        status_message = "Match " + std::to_string(search_ctx.current_match_idx + 1) +
                        "/" + std::to_string(search_ctx.matches.size());
    }

    // 滚动到链接位置
    void scroll_to_link(int link_idx) {
        if (link_idx < 0 || link_idx >= static_cast<int>(current_layout.link_positions.size())) {
            return;
        }

        const auto& pos = current_layout.link_positions[link_idx];
        if (pos.start_line < 0) {
            return;  // 链接位置无效
        }

        int visible_lines = screen_height - 1;
        int link_line = pos.start_line;

        // 确保链接行在可见区域
        if (link_line < scroll_pos) {
            // 链接在视口上方，滚动使其出现在顶部附近
            scroll_pos = std::max(0, link_line - 2);
        } else if (link_line >= scroll_pos + visible_lines) {
            // 链接在视口下方，滚动使其出现在中间
            scroll_pos = link_line - visible_lines / 2;
        }

        int max_scroll = std::max(0, current_layout.total_lines - visible_lines);
        scroll_pos = std::max(0, std::min(scroll_pos, max_scroll));
    }

    void show_help() {
        std::string help_html = R"(
<!DOCTYPE html>
<html>
<head><title>TUT 2.0 Help</title></head>
<body>
<h1>TUT 2.0 - Terminal Browser</h1>

<h2>Navigation</h2>
<ul>
<li>j/k - Scroll down/up</li>
<li>Ctrl+d/Ctrl+u - Page down/up</li>
<li>gg - Go to top</li>
<li>G - Go to bottom</li>
</ul>

<h2>Links</h2>
<ul>
<li>Tab - Next link</li>
<li>Shift+Tab - Previous link</li>
<li>Enter - Follow link</li>
</ul>

<h2>History</h2>
<ul>
<li>h - Go back</li>
<li>l - Go forward</li>
</ul>

<h2>Search</h2>
<ul>
<li>/ - Search forward</li>
<li>n - Next match</li>
<li>N - Previous match</li>
</ul>

<h2>Commands</h2>
<ul>
<li>:o URL - Open URL</li>
<li>:q - Quit</li>
<li>? - Show this help</li>
</ul>

<h2>Forms</h2>
<ul>
<li>Tab - Navigate links and form fields</li>
<li>Enter - Activate link or submit form</li>
</ul>

<hr>
<p>TUT 2.0 - A modern terminal browser with True Color support</p>
</body>
</html>
)";
        current_tree = html_parser.parse_tree(help_html, "help://");
        current_layout = layout_engine->layout(current_tree);
        scroll_pos = 0;
        active_link = current_tree.links.empty() ? -1 : 0;
        status_message = "Help - Press any key to continue";
    }
};

BrowserV2::BrowserV2() : pImpl(std::make_unique<Impl>()) {
    pImpl->input_handler.set_status_callback([this](const std::string& msg) {
        pImpl->status_message = msg;
    });
}

BrowserV2::~BrowserV2() = default;

void BrowserV2::run(const std::string& initial_url) {
    if (!pImpl->init_screen()) {
        throw std::runtime_error("Failed to initialize terminal");
    }

    if (!initial_url.empty()) {
        load_url(initial_url);
    } else {
        pImpl->show_help();
    }

    bool running = true;
    while (running) {
        pImpl->draw_screen();

        int ch = pImpl->terminal.get_key(50);
        if (ch == -1) continue;

        // 处理窗口大小变化
        if (ch == KEY_RESIZE) {
            pImpl->handle_resize();
            continue;
        }

        auto result = pImpl->input_handler.handle_key(ch);
        if (result.action == Action::QUIT) {
            running = false;
        } else if (result.action != Action::NONE) {
            pImpl->handle_action(result);
        }
    }

    pImpl->cleanup_screen();
}

bool BrowserV2::load_url(const std::string& url) {
    return pImpl->load_page(url);
}

std::string BrowserV2::get_current_url() const {
    return pImpl->current_url;
}
