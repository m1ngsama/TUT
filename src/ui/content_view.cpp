/**
 * @file content_view.cpp
 * @brief 内容视图组件实现
 */

#include "ui/content_view.hpp"
#include "core/browser_engine.hpp"

namespace tut {

class ContentView::Impl {
public:
    std::string content_;
    std::vector<LinkInfo> links_;
    int scroll_position_{0};
    int selected_link_{-1};
    std::string search_query_;
    std::vector<int> search_results_;
    int current_search_result_{-1};
    std::function<void(const std::string&)> on_link_activate_;
};

ContentView::ContentView() : impl_(std::make_unique<Impl>()) {}

ContentView::~ContentView() = default;

void ContentView::setContent(const std::string& content) {
    impl_->content_ = content;
    impl_->scroll_position_ = 0;
}

void ContentView::setLinks(const std::vector<LinkInfo>& links) {
    impl_->links_ = links;
    impl_->selected_link_ = links.empty() ? -1 : 0;
}

void ContentView::scrollDown(int lines) {
    impl_->scroll_position_ += lines;
}

void ContentView::scrollUp(int lines) {
    impl_->scroll_position_ = std::max(0, impl_->scroll_position_ - lines);
}

void ContentView::scrollToTop() {
    impl_->scroll_position_ = 0;
}

void ContentView::scrollToBottom() {
    // TODO: 计算最大滚动位置
    impl_->scroll_position_ = 99999;
}

void ContentView::pageDown() {
    scrollDown(20);  // TODO: 根据实际视口大小
}

void ContentView::pageUp() {
    scrollUp(20);
}

int ContentView::getScrollPosition() const {
    return impl_->scroll_position_;
}

void ContentView::selectNextLink() {
    if (impl_->links_.empty()) return;
    impl_->selected_link_ = (impl_->selected_link_ + 1) % static_cast<int>(impl_->links_.size());
}

void ContentView::selectPreviousLink() {
    if (impl_->links_.empty()) return;
    impl_->selected_link_--;
    if (impl_->selected_link_ < 0) {
        impl_->selected_link_ = static_cast<int>(impl_->links_.size()) - 1;
    }
}

int ContentView::getSelectedLinkIndex() const {
    return impl_->selected_link_;
}

void ContentView::onLinkActivate(std::function<void(const std::string&)> callback) {
    impl_->on_link_activate_ = std::move(callback);
}

int ContentView::search(const std::string& query) {
    impl_->search_query_ = query;
    impl_->search_results_.clear();
    impl_->current_search_result_ = -1;

    // TODO: 实现文本搜索
    return static_cast<int>(impl_->search_results_.size());
}

void ContentView::nextSearchResult() {
    if (impl_->search_results_.empty()) return;
    impl_->current_search_result_ =
        (impl_->current_search_result_ + 1) % static_cast<int>(impl_->search_results_.size());
}

void ContentView::previousSearchResult() {
    if (impl_->search_results_.empty()) return;
    impl_->current_search_result_--;
    if (impl_->current_search_result_ < 0) {
        impl_->current_search_result_ = static_cast<int>(impl_->search_results_.size()) - 1;
    }
}

void ContentView::clearSearch() {
    impl_->search_query_.clear();
    impl_->search_results_.clear();
    impl_->current_search_result_ = -1;
}

}  // namespace tut
