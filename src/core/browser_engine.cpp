/**
 * @file browser_engine.cpp
 * @brief 浏览器引擎实现
 */

#include "core/browser_engine.hpp"

namespace tut {

class BrowserEngine::Impl {
public:
    std::string current_url_;
    std::string title_;
    std::string content_;
    std::vector<LinkInfo> links_;
    std::vector<std::string> history_;
    size_t history_index_{0};
};

BrowserEngine::BrowserEngine() : impl_(std::make_unique<Impl>()) {}

BrowserEngine::~BrowserEngine() = default;

bool BrowserEngine::loadUrl(const std::string& url) {
    // TODO: 实现 HTTP 请求和 HTML 解析
    impl_->current_url_ = url;
    return true;
}

bool BrowserEngine::loadHtml(const std::string& html) {
    // TODO: 实现 HTML 解析
    impl_->content_ = html;
    return true;
}

std::string BrowserEngine::getTitle() const {
    return impl_->title_;
}

std::string BrowserEngine::getCurrentUrl() const {
    return impl_->current_url_;
}

std::vector<LinkInfo> BrowserEngine::extractLinks() const {
    return impl_->links_;
}

std::string BrowserEngine::getRenderedContent() const {
    return impl_->content_;
}

bool BrowserEngine::goBack() {
    if (!canGoBack()) return false;
    impl_->history_index_--;
    return loadUrl(impl_->history_[impl_->history_index_]);
}

bool BrowserEngine::goForward() {
    if (!canGoForward()) return false;
    impl_->history_index_++;
    return loadUrl(impl_->history_[impl_->history_index_]);
}

bool BrowserEngine::refresh() {
    return loadUrl(impl_->current_url_);
}

bool BrowserEngine::canGoBack() const {
    return impl_->history_index_ > 0;
}

bool BrowserEngine::canGoForward() const {
    return impl_->history_index_ < impl_->history_.size() - 1;
}

}  // namespace tut
