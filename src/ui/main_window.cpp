/**
 * @file main_window.cpp
 * @brief 主窗口实现
 */

#include "ui/main_window.hpp"
#include "ui/content_view.hpp"

#include <ftxui/component/component.hpp>
#include <ftxui/component/screen_interactive.hpp>
#include <ftxui/dom/elements.hpp>

#include <sstream>
#include <algorithm>

namespace tut {

class MainWindow::Impl {
public:
    std::string url_;
    std::string title_;
    std::vector<DisplayLink> links_;
    int scroll_offset_{0};
    int selected_link_{-1};
    int viewport_height_{20};

    std::string status_message_;
    bool loading_{false};
    bool can_go_back_{false};
    bool can_go_forward_{false};

    double load_time_{0.0};
    size_t load_bytes_{0};
    int link_count_{0};

    std::function<void(const std::string&)> on_navigate_;
    std::function<void(WindowEvent)> on_event_;
    std::function<void(int)> on_link_click_;

    // Split content into lines for scrolling
    std::vector<std::string> content_lines_;

    void setContent(const std::string& content) {
        content_lines_.clear();
        std::istringstream iss(content);
        std::string line;
        while (std::getline(iss, line)) {
            content_lines_.push_back(line);
        }
        scroll_offset_ = 0;
    }

    void scrollDown(int lines = 1) {
        int max_scroll = std::max(0, static_cast<int>(content_lines_.size()) - viewport_height_);
        scroll_offset_ = std::min(scroll_offset_ + lines, max_scroll);
    }

    void scrollUp(int lines = 1) {
        scroll_offset_ = std::max(0, scroll_offset_ - lines);
    }

    void scrollToTop() {
        scroll_offset_ = 0;
    }

    void scrollToBottom() {
        scroll_offset_ = std::max(0, static_cast<int>(content_lines_.size()) - viewport_height_);
    }

    void selectNextLink() {
        if (links_.empty()) return;
        selected_link_ = (selected_link_ + 1) % static_cast<int>(links_.size());
    }

    void selectPreviousLink() {
        if (links_.empty()) return;
        selected_link_--;
        if (selected_link_ < 0) {
            selected_link_ = static_cast<int>(links_.size()) - 1;
        }
    }
};

MainWindow::MainWindow() : impl_(std::make_unique<Impl>()) {}

MainWindow::~MainWindow() = default;

bool MainWindow::init() {
    return true;
}

int MainWindow::run() {
    using namespace ftxui;

    auto screen = ScreenInteractive::Fullscreen();

    // 地址栏输入
    std::string address_content = impl_->url_;
    auto address_input = Input(&address_content, "Enter URL...");
    bool address_focused = false;

    // 内容渲染器
    auto content_renderer = Renderer([this] {
        Elements lines;

        // Title
        if (!impl_->title_.empty()) {
            lines.push_back(text(impl_->title_) | bold | center);
            lines.push_back(separator());
        }

        // Content with scrolling
        int start = impl_->scroll_offset_;
        int end = std::min(start + impl_->viewport_height_,
                          static_cast<int>(impl_->content_lines_.size()));

        for (int i = start; i < end; i++) {
            lines.push_back(text(impl_->content_lines_[i]));
        }

        // Scroll indicator
        if (!impl_->content_lines_.empty()) {
            int total_lines = static_cast<int>(impl_->content_lines_.size());
            std::string scroll_info = "Lines " + std::to_string(start + 1) +
                                     "-" + std::to_string(end) +
                                     " / " + std::to_string(total_lines);
            lines.push_back(separator());
            lines.push_back(text(scroll_info) | dim | align_right);
        }

        return vbox(lines) | flex;
    });

    // 状态面板
    auto status_panel = Renderer([this] {
        Elements status_items;

        if (impl_->loading_) {
            status_items.push_back(text("⏳ Loading...") | dim);
        } else if (impl_->load_time_ > 0) {
            std::string stats = "⬇ " + std::to_string(impl_->load_bytes_ / 1024) + " KB  " +
                               "🕐 " + std::to_string(static_cast<int>(impl_->load_time_ * 1000)) + "ms  " +
                               "🔗 " + std::to_string(impl_->link_count_) + " links";
            status_items.push_back(text(stats) | dim);
        } else {
            status_items.push_back(text("Ready") | dim);
        }

        if (impl_->selected_link_ >= 0 && impl_->selected_link_ < static_cast<int>(impl_->links_.size())) {
            status_items.push_back(separator());
            std::string link_info = "[" + std::to_string(impl_->selected_link_ + 1) + "] " +
                                   impl_->links_[impl_->selected_link_].url;
            status_items.push_back(text(link_info) | dim);
        }

        return hbox(status_items);
    });

    // 主布局
    auto main_renderer = Renderer([&] {
        return vbox({
            // 顶部栏
            hbox({
                text(impl_->can_go_back_ ? "[◀]" : "[◀]") | (impl_->can_go_back_ ? bold : dim),
                text(" "),
                text(impl_->can_go_forward_ ? "[▶]" : "[▶]") | (impl_->can_go_forward_ ? bold : dim),
                text(" "),
                text("[⟳]") | bold,
                text(" "),
                address_input->Render() | flex | border | (address_focused ? focus : select),
                text(" "),
                text("[⚙]") | bold,
                text(" "),
                text("[?]") | bold,
            }),
            separator(),
            // 内容区
            content_renderer->Render() | flex,
            separator(),
            // 底部面板
            hbox({
                vbox({
                    text("📑 Bookmarks") | bold,
                    text("  (empty)") | dim,
                }) | flex,
                separator(),
                vbox({
                    text("📊 Status") | bold,
                    status_panel->Render(),
                }) | flex,
            }),
            separator(),
            // 状态栏
            hbox({
                text("[F1]Help") | dim,
                text(" "),
                text("[F2]Bookmarks") | dim,
                text(" "),
                text("[F3]History") | dim,
                text(" "),
                text("[F10]Quit") | dim,
                filler(),
                text(impl_->status_message_) | dim,
            }),
        }) | border;
    });

    // 事件处理
    main_renderer |= CatchEvent([&](Event event) {
        // Quit
        if (event == Event::Escape || event == Event::Character('q') ||
            event == Event::F10) {
            screen.ExitLoopClosure()();
            return true;
        }

        // Address bar focus (use 'o' key instead of Ctrl+L)
        if (event == Event::Character('o') && !address_focused) {
            address_focused = true;
            return true;
        }

        // Navigate from address bar
        if (event == Event::Return && address_focused) {
            if (impl_->on_navigate_) {
                impl_->on_navigate_(address_content);
                address_focused = false;
            }
            return true;
        }

        // Exit address bar
        if (event == Event::Escape && address_focused) {
            address_focused = false;
            return true;
        }

        // Don't handle other keys if address bar is focused
        if (address_focused) {
            return false;
        }

        // Scrolling
        if (event == Event::Character('j') || event == Event::ArrowDown) {
            impl_->scrollDown(1);
            return true;
        }
        if (event == Event::Character('k') || event == Event::ArrowUp) {
            impl_->scrollUp(1);
            return true;
        }
        if (event == Event::Character(' ') || event == Event::PageDown) {
            impl_->scrollDown(impl_->viewport_height_ - 2);
            return true;
        }
        if (event == Event::Character('b') || event == Event::PageUp) {
            impl_->scrollUp(impl_->viewport_height_ - 2);
            return true;
        }
        if (event == Event::Character('g')) {
            impl_->scrollToTop();
            return true;
        }
        if (event == Event::Character('G')) {
            impl_->scrollToBottom();
            return true;
        }

        // Link navigation
        if (event == Event::Tab) {
            impl_->selectNextLink();
            return true;
        }
        if (event == Event::TabReverse) {
            impl_->selectPreviousLink();
            return true;
        }

        // Follow link
        if (event == Event::Return) {
            if (impl_->selected_link_ >= 0 &&
                impl_->selected_link_ < static_cast<int>(impl_->links_.size())) {
                if (impl_->on_link_click_) {
                    impl_->on_link_click_(impl_->selected_link_);
                }
            }
            return true;
        }

        // Number shortcuts (1-9)
        if (event.is_character()) {
            char c = event.character()[0];
            if (c >= '1' && c <= '9') {
                int link_idx = c - '1';
                if (link_idx < static_cast<int>(impl_->links_.size())) {
                    impl_->selected_link_ = link_idx;
                    if (impl_->on_link_click_) {
                        impl_->on_link_click_(link_idx);
                    }
                }
                return true;
            }
        }

        // Back/Forward
        if (event == Event::Backspace && impl_->can_go_back_) {
            if (impl_->on_event_) {
                impl_->on_event_(WindowEvent::Back);
            }
            return true;
        }

        // Refresh
        if (event == Event::Character('r') || event == Event::F5) {
            if (impl_->on_event_) {
                impl_->on_event_(WindowEvent::Refresh);
            }
            return true;
        }

        return false;
    });

    screen.Loop(main_renderer);
    return 0;
}

void MainWindow::setStatusMessage(const std::string& message) {
    impl_->status_message_ = message;
}

void MainWindow::setUrl(const std::string& url) {
    impl_->url_ = url;
}

void MainWindow::setTitle(const std::string& title) {
    impl_->title_ = title;
}

void MainWindow::setContent(const std::string& content) {
    impl_->setContent(content);
}

void MainWindow::setLoading(bool loading) {
    impl_->loading_ = loading;
}

void MainWindow::setLinks(const std::vector<DisplayLink>& links) {
    impl_->links_ = links;
    impl_->selected_link_ = links.empty() ? -1 : 0;
}

void MainWindow::setBookmarks(const std::vector<DisplayBookmark>& /*bookmarks*/) {
    // TODO: Implement bookmark display
}

void MainWindow::setHistory(const std::vector<DisplayBookmark>& /*history*/) {
    // TODO: Implement history display
}

void MainWindow::setCanGoBack(bool can) {
    impl_->can_go_back_ = can;
}

void MainWindow::setCanGoForward(bool can) {
    impl_->can_go_forward_ = can;
}

void MainWindow::setLoadStats(double elapsed_seconds, size_t bytes, int link_count) {
    impl_->load_time_ = elapsed_seconds;
    impl_->load_bytes_ = bytes;
    impl_->link_count_ = link_count;
}

void MainWindow::onNavigate(std::function<void(const std::string&)> callback) {
    impl_->on_navigate_ = std::move(callback);
}

void MainWindow::onEvent(std::function<void(WindowEvent)> callback) {
    impl_->on_event_ = std::move(callback);
}

void MainWindow::onLinkClick(std::function<void(int index)> callback) {
    impl_->on_link_click_ = std::move(callback);
}

void MainWindow::onBookmarkClick(std::function<void(const std::string& url)> /*callback*/) {
    // TODO: Implement bookmark click callback
}

}  // namespace tut
