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
#include <cctype>

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

    // Search state
    bool search_mode_{false};
    std::string search_query_;
    std::vector<int> search_matches_;  // Line indices with matches
    int current_match_{-1};  // Index into search_matches_

    // Bookmark state
    bool bookmark_panel_visible_{false};
    std::vector<DisplayBookmark> bookmarks_;
    int selected_bookmark_{-1};

    // History state
    bool history_panel_visible_{false};
    std::vector<DisplayBookmark> history_;
    int selected_history_{-1};

    void setContent(const std::string& content) {
        content_lines_.clear();
        std::istringstream iss(content);
        std::string line;
        while (std::getline(iss, line)) {
            content_lines_.push_back(line);
        }
        scroll_offset_ = 0;

        // Clear search state when new content is loaded
        search_mode_ = false;
        search_query_.clear();
        search_matches_.clear();
        current_match_ = -1;
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

    void executeSearch() {
        search_matches_.clear();
        current_match_ = -1;

        if (search_query_.empty()) {
            return;
        }

        // Convert search query to lowercase for case-insensitive search
        std::string query_lower = search_query_;
        std::transform(query_lower.begin(), query_lower.end(), query_lower.begin(), ::tolower);

        // Find all lines containing the search query
        for (size_t i = 0; i < content_lines_.size(); ++i) {
            std::string line_lower = content_lines_[i];
            std::transform(line_lower.begin(), line_lower.end(), line_lower.begin(), ::tolower);

            if (line_lower.find(query_lower) != std::string::npos) {
                search_matches_.push_back(static_cast<int>(i));
            }
        }

        if (!search_matches_.empty()) {
            current_match_ = 0;
            // Scroll to first match
            scroll_offset_ = search_matches_[0];
        }
    }

    void nextMatch() {
        if (search_matches_.empty()) return;

        current_match_ = (current_match_ + 1) % static_cast<int>(search_matches_.size());
        // Scroll to the match
        scroll_offset_ = search_matches_[current_match_];
    }

    void previousMatch() {
        if (search_matches_.empty()) return;

        current_match_--;
        if (current_match_ < 0) {
            current_match_ = static_cast<int>(search_matches_.size()) - 1;
        }
        // Scroll to the match
        scroll_offset_ = search_matches_[current_match_];
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

    // 搜索输入
    std::string search_content;
    auto search_input = Input(&search_content, "Search...");
    bool search_focused = false;

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
            // Check if this line is a search match
            bool is_match = false;
            bool is_current_match = false;

            if (!impl_->search_matches_.empty()) {
                auto it = std::find(impl_->search_matches_.begin(),
                                  impl_->search_matches_.end(), i);
                if (it != impl_->search_matches_.end()) {
                    is_match = true;
                    int match_index = std::distance(impl_->search_matches_.begin(), it);
                    is_current_match = (match_index == impl_->current_match_);
                }
            }

            // Render line with appropriate highlighting
            auto line_element = text(impl_->content_lines_[i]);
            if (is_current_match) {
                line_element = line_element | bgcolor(Color::Yellow) | color(Color::Black);
            } else if (is_match) {
                line_element = line_element | bgcolor(Color::Blue) | color(Color::White);
            }
            lines.push_back(line_element);
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

        // Search status takes priority
        if (!impl_->search_matches_.empty()) {
            std::string search_info = "🔍 Match " +
                                     std::to_string(impl_->current_match_ + 1) + "/" +
                                     std::to_string(impl_->search_matches_.size()) +
                                     " for \"" + impl_->search_query_ + "\"";
            status_items.push_back(text(search_info) | color(Color::Yellow));
        } else if (impl_->search_mode_ && !impl_->search_query_.empty()) {
            status_items.push_back(text("🔍 No matches for \"" + impl_->search_query_ + "\"") | dim);
        } else if (impl_->loading_) {
            status_items.push_back(text("⏳ Loading...") | dim);
        } else if (impl_->load_time_ > 0) {
            std::string stats = "⬇ " + std::to_string(impl_->load_bytes_ / 1024) + " KB  " +
                               "🕐 " + std::to_string(static_cast<int>(impl_->load_time_ * 1000)) + "ms  " +
                               "🔗 " + std::to_string(impl_->link_count_) + " links";
            status_items.push_back(text(stats) | dim);
        } else {
            status_items.push_back(text("Ready") | dim);
        }

        if (impl_->selected_link_ >= 0 && impl_->selected_link_ < static_cast<int>(impl_->links_.size()) &&
            impl_->search_matches_.empty()) {
            status_items.push_back(separator());
            std::string link_info = "[" + std::to_string(impl_->selected_link_ + 1) + "] " +
                                   impl_->links_[impl_->selected_link_].url;
            status_items.push_back(text(link_info) | dim);
        }

        return hbox(status_items);
    });

    // 主布局
    auto main_renderer = Renderer([&] {
        Elements top_bar_elements;

        if (search_focused) {
            // Show search bar when in search mode
            top_bar_elements.push_back(hbox({
                text("🔍 "),
                search_input->Render() | flex | border | focus,
                text(" [Esc to cancel]") | dim,
            }));
        } else {
            // Normal address bar
            top_bar_elements.push_back(hbox({
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
            }));
        }

        return vbox({
            // 顶部栏
            vbox(top_bar_elements),
            separator(),
            // 内容区
            content_renderer->Render() | flex,
            separator(),
            // 底部面板
            hbox({
                vbox({
                    text("📑 Bookmarks") | bold,
                    [this]() -> Element {
                        if (!impl_->bookmarks_.empty()) {
                            Elements bookmark_lines;
                            int max_display = 5;  // Show up to 5 bookmarks
                            int end = std::min(max_display, static_cast<int>(impl_->bookmarks_.size()));
                            for (int i = 0; i < end; i++) {
                                const auto& bm = impl_->bookmarks_[i];
                                auto line = text("  [" + std::to_string(i + 1) + "] " + bm.title);
                                if (i == impl_->selected_bookmark_) {
                                    line = line | bold | color(Color::Yellow);
                                } else {
                                    line = line | dim;
                                }
                                bookmark_lines.push_back(line);
                            }
                            if (impl_->bookmarks_.size() > static_cast<size_t>(max_display)) {
                                bookmark_lines.push_back(
                                    text("  +" + std::to_string(impl_->bookmarks_.size() - max_display) + " more...") | dim
                                );
                            }
                            return vbox(bookmark_lines);
                        } else {
                            return text("  (empty)") | dim;
                        }
                    }()
                }) | flex,
                separator(),
                vbox({
                    text("📚 History") | bold,
                    [this]() -> Element {
                        if (!impl_->history_.empty()) {
                            Elements history_lines;
                            int max_display = 5;  // Show up to 5 history entries
                            int end = std::min(max_display, static_cast<int>(impl_->history_.size()));
                            for (int i = 0; i < end; i++) {
                                const auto& entry = impl_->history_[i];
                                auto line = text("  [" + std::to_string(i + 1) + "] " + entry.title);
                                if (i == impl_->selected_history_) {
                                    line = line | bold | color(Color::Cyan);
                                } else {
                                    line = line | dim;
                                }
                                history_lines.push_back(line);
                            }
                            if (impl_->history_.size() > static_cast<size_t>(max_display)) {
                                history_lines.push_back(
                                    text("  +" + std::to_string(impl_->history_.size() - max_display) + " more...") | dim
                                );
                            }
                            return vbox(history_lines);
                        } else {
                            return text("  (empty)") | dim;
                        }
                    }()
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

        // Search mode activation
        if (event == Event::Character('/') && !address_focused && !search_focused) {
            search_focused = true;
            search_content.clear();
            return true;
        }

        // Execute search
        if (event == Event::Return && search_focused) {
            impl_->search_mode_ = true;
            impl_->search_query_ = search_content;
            impl_->executeSearch();
            search_focused = false;
            return true;
        }

        // Exit search mode
        if (event == Event::Escape && search_focused) {
            search_focused = false;
            search_content.clear();
            return true;
        }

        // Don't handle other keys if address bar or search is focused
        if (address_focused || search_focused) {
            return false;
        }

        // Navigate search results (only when not in input mode)
        if (event == Event::Character('n') && !impl_->search_matches_.empty()) {
            impl_->nextMatch();
            return true;
        }
        if (event == Event::Character('N') && !impl_->search_matches_.empty()) {
            impl_->previousMatch();
            return true;
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
        if (event == Event::Character('f') && impl_->can_go_forward_) {
            if (impl_->on_event_) {
                impl_->on_event_(WindowEvent::Forward);
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

        // Add bookmark (Ctrl+D)
        if (event == Event::Special("\x04")) {  // Ctrl+D
            if (impl_->on_event_) {
                impl_->on_event_(WindowEvent::AddBookmark);
            }
            return true;
        }

        // Toggle bookmark panel (F2)
        if (event == Event::F2) {
            impl_->bookmark_panel_visible_ = !impl_->bookmark_panel_visible_;
            if (impl_->on_event_) {
                impl_->on_event_(WindowEvent::OpenBookmarks);
            }
            return true;
        }

        // Toggle history panel (F3)
        if (event == Event::F3) {
            impl_->history_panel_visible_ = !impl_->history_panel_visible_;
            if (impl_->on_event_) {
                impl_->on_event_(WindowEvent::OpenHistory);
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

void MainWindow::setBookmarks(const std::vector<DisplayBookmark>& bookmarks) {
    impl_->bookmarks_ = bookmarks;
    impl_->selected_bookmark_ = bookmarks.empty() ? -1 : 0;
}

void MainWindow::setHistory(const std::vector<DisplayBookmark>& history) {
    impl_->history_ = history;
    impl_->selected_history_ = history.empty() ? -1 : 0;
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
