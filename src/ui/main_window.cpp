/**
 * @file main_window.cpp
 * @brief 主窗口实现
 */

#include "ui/main_window.hpp"

#include <ftxui/component/component.hpp>
#include <ftxui/component/screen_interactive.hpp>
#include <ftxui/dom/elements.hpp>

namespace tut {

class MainWindow::Impl {
public:
    std::string url_;
    std::string title_;
    std::string content_;
    std::string status_message_;
    bool loading_{false};

    std::function<void(const std::string&)> on_navigate_;
    std::function<void(WindowEvent)> on_event_;
};

MainWindow::MainWindow() : impl_(std::make_unique<Impl>()) {}

MainWindow::~MainWindow() = default;

bool MainWindow::init() {
    // TODO: 初始化 FTXUI 组件
    return true;
}

int MainWindow::run() {
    using namespace ftxui;

    auto screen = ScreenInteractive::Fullscreen();

    // 地址栏输入
    std::string address_content = impl_->url_;
    auto address_input = Input(&address_content, "Enter URL...");

    // 内容区域
    auto content_renderer = Renderer([this] {
        return vbox({
            text(impl_->title_) | bold | center,
            separator(),
            paragraph(impl_->content_),
        }) | flex;
    });

    // 状态栏
    auto status_renderer = Renderer([this] {
        std::string status = impl_->loading_ ? "Loading..." : impl_->status_message_;
        return text(status) | dim;
    });

    // 主布局
    auto main_layout = Container::Vertical({
        address_input,
        content_renderer,
        status_renderer,
    });

    auto main_renderer = Renderer(main_layout, [&] {
        return vbox({
            // 顶部栏
            hbox({
                text("[◀]") | bold,
                text(" "),
                text("[▶]") | bold,
                text(" "),
                text("[⟳]") | bold,
                text(" "),
                address_input->Render() | flex | border,
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
                    text("  Ready") | dim,
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
                status_renderer->Render(),
            }),
        }) | border;
    });

    // 事件处理
    main_renderer |= CatchEvent([&](Event event) {
        if (event == Event::Escape || event == Event::Character('q')) {
            screen.ExitLoopClosure()();
            return true;
        }
        if (event == Event::Return) {
            if (impl_->on_navigate_) {
                impl_->on_navigate_(address_content);
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
    impl_->content_ = content;
}

void MainWindow::setLoading(bool loading) {
    impl_->loading_ = loading;
}

void MainWindow::onNavigate(std::function<void(const std::string&)> callback) {
    impl_->on_navigate_ = std::move(callback);
}

void MainWindow::onEvent(std::function<void(WindowEvent)> callback) {
    impl_->on_event_ = std::move(callback);
}

}  // namespace tut
