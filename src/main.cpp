#include "browser.h"
#include <iostream>
#include <cstring>

void print_usage(const char* prog_name) {
    std::cout << "TUT - Terminal User Interface Browser\n"
              << "A vim-style terminal web browser for comfortable reading\n\n"
              << "Usage: " << prog_name << " [URL]\n\n"
              << "If no URL is provided, the browser will start with a help page.\n\n"
              << "Examples:\n"
              << "  " << prog_name << "\n"
              << "  " << prog_name << " https://example.com\n"
              << "  " << prog_name << " https://news.ycombinator.com\n\n"
              << "Vim-style keybindings:\n"
              << "  j/k     - Scroll down/up\n"
              << "  gg/G    - Go to top/bottom\n"
              << "  /       - Search\n"
              << "  Tab     - Next link\n"
              << "  Enter   - Follow link\n"
              << "  h/l     - Back/Forward\n"
              << "  :o URL  - Open URL\n"
              << "  :q      - Quit\n"
              << "  ?       - Show help\n";
}

int main(int argc, char* argv[]) {
    // 解析命令行参数
    std::string initial_url;

    if (argc > 1) {
        if (std::strcmp(argv[1], "-h") == 0 || std::strcmp(argv[1], "--help") == 0) {
            print_usage(argv[0]);
            return 0;
        }
        initial_url = argv[1];
    }

    try {
        Browser browser;
        browser.run(initial_url);
    } catch (const std::exception& e) {
        std::cerr << "Error: " << e.what() << std::endl;
        return 1;
    }

    return 0;
}

