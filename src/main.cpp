/**
 * @file main.cpp
 * @brief TUT 程序入口
 * @author m1ngsama
 * @date 2024-12-29
 */

#include <iostream>
#include <string>
#include <cstring>
#include <chrono>

#include "tut/version.hpp"
#include "core/browser_engine.hpp"
#include "ui/main_window.hpp"
#include "utils/logger.hpp"
#include "utils/config.hpp"
#include "utils/theme.hpp"

namespace {

void printVersion() {
    std::cout << tut::PROJECT_NAME << " " << tut::VERSION_STRING << "\n"
              << tut::PROJECT_DESCRIPTION << "\n"
              << "Homepage: " << tut::PROJECT_HOMEPAGE << "\n"
              << "Built with " << tut::COMPILER_ID << " " << tut::COMPILER_VERSION << "\n";
}

void printHelp(const char* prog_name) {
    std::cout << "TUT - Terminal UI Textual Browser\n"
              << "A lightweight terminal browser with btop-style interface\n\n"
              << "Usage: " << prog_name << " [OPTIONS] [URL]\n\n"
              << "Options:\n"
              << "  -h, --help     Show this help message\n"
              << "  -v, --version  Show version information\n"
              << "  -c, --config   Specify config file path\n"
              << "  -t, --theme    Specify theme name\n"
              << "  -d, --debug    Enable debug logging\n\n"
              << "Examples:\n"
              << "  " << prog_name << "\n"
              << "  " << prog_name << " https://example.com\n"
              << "  " << prog_name << " --theme nord https://github.com\n\n"
              << "Keyboard shortcuts:\n"
              << "  j/k, ↓/↑     Scroll down/up\n"
              << "  g/G          Go to top/bottom\n"
              << "  Space        Page down\n"
              << "  b            Page up\n"
              << "  Tab          Next link\n"
              << "  Shift+Tab    Previous link\n"
              << "  Enter        Follow link\n"
              << "  Backspace    Go back\n"
              << "  /            Search in page\n"
              << "  n/N          Next/previous search result\n"
              << "  Ctrl+L       Focus address bar\n"
              << "  F1           Help\n"
              << "  F2           Bookmarks\n"
              << "  F3           History\n"
              << "  F5, r        Refresh\n"
              << "  Ctrl+D       Add bookmark\n"
              << "  Ctrl+Q, F10  Quit\n";
}

}  // namespace

int main(int argc, char* argv[]) {
    using namespace tut;

    std::string initial_url;
    std::string config_file;
    std::string theme_name;
    bool debug_mode = false;

    // 解析命令行参数
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "-h") == 0 || std::strcmp(argv[i], "--help") == 0) {
            printHelp(argv[0]);
            return 0;
        }
        if (std::strcmp(argv[i], "-v") == 0 || std::strcmp(argv[i], "--version") == 0) {
            printVersion();
            return 0;
        }
        if (std::strcmp(argv[i], "-d") == 0 || std::strcmp(argv[i], "--debug") == 0) {
            debug_mode = true;
            continue;
        }
        if ((std::strcmp(argv[i], "-c") == 0 || std::strcmp(argv[i], "--config") == 0) &&
            i + 1 < argc) {
            config_file = argv[++i];
            continue;
        }
        if ((std::strcmp(argv[i], "-t") == 0 || std::strcmp(argv[i], "--theme") == 0) &&
            i + 1 < argc) {
            theme_name = argv[++i];
            continue;
        }
        // 假定其他参数是 URL
        if (argv[i][0] != '-') {
            initial_url = argv[i];
        }
    }

    // 初始化日志系统
    Logger& logger = Logger::instance();
    logger.setLevel(debug_mode ? LogLevel::Debug : LogLevel::Info);

    LOG_INFO << "Starting TUT " << VERSION_STRING;

    // 加载配置
    Config& config = Config::instance();
    if (!config_file.empty()) {
        config.load(config_file);
    } else {
        std::string default_config = config.getConfigPath() + "/config.toml";
        config.load(default_config);
    }

    // 加载主题
    ThemeManager& theme_manager = ThemeManager::instance();
    theme_manager.loadThemesFromDirectory(config.getConfigPath() + "/themes");

    if (!theme_name.empty()) {
        if (!theme_manager.setTheme(theme_name)) {
            LOG_WARN << "Theme not found: " << theme_name << ", using default";
        }
    } else {
        theme_manager.setTheme(config.getDefaultTheme());
    }

    try {
        // 创建浏览器引擎
        BrowserEngine engine;

        // 创建主窗口
        MainWindow window;

        // 设置导航回调
        window.onNavigate([&engine, &window](const std::string& url) {
            LOG_INFO << "Navigating to: " << url;
            window.setLoading(true);

            auto start_time = std::chrono::steady_clock::now();

            if (engine.loadUrl(url)) {
                auto end_time = std::chrono::steady_clock::now();
                double elapsed = std::chrono::duration<double>(end_time - start_time).count();

                // Update window content
                window.setTitle(engine.getTitle());
                window.setContent(engine.getRenderedContent());
                window.setUrl(url);

                // Convert LinkInfo to DisplayLink
                std::vector<DisplayLink> display_links;
                for (const auto& link : engine.extractLinks()) {
                    DisplayLink dl;
                    dl.text = link.text;
                    dl.url = link.url;
                    dl.visited = false;
                    display_links.push_back(dl);
                }
                window.setLinks(display_links);

                // Update navigation state
                window.setCanGoBack(engine.canGoBack());
                window.setCanGoForward(engine.canGoForward());

                // Update stats (assuming response body size)
                size_t content_size = engine.getRenderedContent().size();
                window.setLoadStats(elapsed, content_size, static_cast<int>(display_links.size()));

                window.setStatusMessage("Loaded: " + url);
            } else {
                window.setStatusMessage("Failed to load: " + url);
            }

            window.setLoading(false);
        });

        // 设置链接点击回调
        window.onLinkClick([&engine, &window](int index) {
            auto links = engine.extractLinks();
            if (index >= 0 && index < static_cast<int>(links.size())) {
                const std::string& link_url = links[index].url;
                LOG_INFO << "Following link [" << index + 1 << "]: " << link_url;

                // Trigger navigation
                window.setLoading(true);

                auto start_time = std::chrono::steady_clock::now();

                if (engine.loadUrl(link_url)) {
                    auto end_time = std::chrono::steady_clock::now();
                    double elapsed = std::chrono::duration<double>(end_time - start_time).count();

                    window.setTitle(engine.getTitle());
                    window.setContent(engine.getRenderedContent());
                    window.setUrl(link_url);

                    // Convert LinkInfo to DisplayLink
                    std::vector<DisplayLink> display_links;
                    for (const auto& link : engine.extractLinks()) {
                        DisplayLink dl;
                        dl.text = link.text;
                        dl.url = link.url;
                        dl.visited = false;
                        display_links.push_back(dl);
                    }
                    window.setLinks(display_links);

                    window.setCanGoBack(engine.canGoBack());
                    window.setCanGoForward(engine.canGoForward());

                    size_t content_size = engine.getRenderedContent().size();
                    window.setLoadStats(elapsed, content_size, static_cast<int>(display_links.size()));

                    window.setStatusMessage("Loaded: " + link_url);
                } else {
                    window.setStatusMessage("Failed to load: " + link_url);
                }

                window.setLoading(false);
            }
        });

        // 设置窗口事件回调
        window.onEvent([&engine, &window](WindowEvent event) {
            switch (event) {
                case WindowEvent::Back:
                    if (engine.goBack()) {
                        window.setTitle(engine.getTitle());
                        window.setContent(engine.getRenderedContent());
                        window.setUrl(engine.getCurrentUrl());

                        std::vector<DisplayLink> display_links;
                        for (const auto& link : engine.extractLinks()) {
                            DisplayLink dl;
                            dl.text = link.text;
                            dl.url = link.url;
                            dl.visited = false;
                            display_links.push_back(dl);
                        }
                        window.setLinks(display_links);

                        window.setCanGoBack(engine.canGoBack());
                        window.setCanGoForward(engine.canGoForward());
                    }
                    break;
                case WindowEvent::Forward:
                    if (engine.goForward()) {
                        window.setTitle(engine.getTitle());
                        window.setContent(engine.getRenderedContent());
                        window.setUrl(engine.getCurrentUrl());

                        std::vector<DisplayLink> display_links;
                        for (const auto& link : engine.extractLinks()) {
                            DisplayLink dl;
                            dl.text = link.text;
                            dl.url = link.url;
                            dl.visited = false;
                            display_links.push_back(dl);
                        }
                        window.setLinks(display_links);

                        window.setCanGoBack(engine.canGoBack());
                        window.setCanGoForward(engine.canGoForward());
                    }
                    break;
                case WindowEvent::Refresh:
                    if (engine.refresh()) {
                        window.setTitle(engine.getTitle());
                        window.setContent(engine.getRenderedContent());

                        std::vector<DisplayLink> display_links;
                        for (const auto& link : engine.extractLinks()) {
                            DisplayLink dl;
                            dl.text = link.text;
                            dl.url = link.url;
                            dl.visited = false;
                            display_links.push_back(dl);
                        }
                        window.setLinks(display_links);
                    }
                    break;
                default:
                    break;
            }
        });

        // 初始化窗口
        if (!window.init()) {
            LOG_FATAL << "Failed to initialize window";
            return 1;
        }

        // 加载初始 URL
        if (!initial_url.empty()) {
            engine.loadUrl(initial_url);
            window.setUrl(initial_url);
            window.setTitle(engine.getTitle());
            window.setContent(engine.getRenderedContent());

            std::vector<DisplayLink> display_links;
            for (const auto& link : engine.extractLinks()) {
                DisplayLink dl;
                dl.text = link.text;
                dl.url = link.url;
                dl.visited = false;
                display_links.push_back(dl);
            }
            window.setLinks(display_links);
            window.setCanGoBack(engine.canGoBack());
            window.setCanGoForward(engine.canGoForward());
        } else {
            window.setUrl("about:blank");
            window.setTitle("TUT - Terminal UI Textual Browser");
            window.setContent("Welcome to TUT!\n\nPress Ctrl+L to enter a URL.");
        }

        // 运行主循环
        int exit_code = window.run();

        LOG_INFO << "TUT exiting with code " << exit_code;
        return exit_code;

    } catch (const std::exception& e) {
        LOG_FATAL << "Fatal error: " << e.what();
        std::cerr << "Error: " << e.what() << std::endl;
        return 1;
    }
}
