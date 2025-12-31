/**
 * @file browser_engine.cpp
 * @brief 浏览器引擎实现
 */

#include "core/browser_engine.hpp"
#include "core/http_client.hpp"
#include "renderer/html_renderer.hpp"

namespace tut {

class BrowserEngine::Impl {
public:
    std::string current_url_;
    std::string title_;
    std::string content_;
    std::vector<LinkInfo> links_;
    std::vector<std::string> history_;
    size_t history_index_{0};

    HttpClient http_client_;
    HtmlRenderer renderer_;
};

BrowserEngine::BrowserEngine() : impl_(std::make_unique<Impl>()) {}

BrowserEngine::~BrowserEngine() = default;

bool BrowserEngine::loadUrl(const std::string& url) {
    impl_->current_url_ = url;

    // 发送 HTTP 请求
    auto response = impl_->http_client_.get(url);

    if (!response.isSuccess()) {
        // 加载失败，设置错误内容
        impl_->title_ = "Error";
        impl_->content_ = "Failed to load page: " +
                          (response.error.empty()
                           ? "HTTP " + std::to_string(response.status_code)
                           : response.error);
        impl_->links_.clear();
        return false;
    }

    // 渲染 HTML
    return loadHtml(response.body);
}

bool BrowserEngine::loadHtml(const std::string& html) {
    // 渲染 HTML
    RenderOptions options;
    options.show_links = true;
    options.use_colors = true;

    auto result = impl_->renderer_.render(html, options);

    impl_->title_ = result.title;
    impl_->content_ = result.text;
    impl_->links_ = result.links;

    // 解析相对 URL
    if (!impl_->current_url_.empty()) {
        for (auto& link : impl_->links_) {
            if (link.url.find("://") == std::string::npos) {
                // 简单的相对 URL 解析
                if (!link.url.empty() && link.url[0] == '/') {
                    // 绝对路径
                    size_t scheme_end = impl_->current_url_.find("://");
                    if (scheme_end != std::string::npos) {
                        size_t host_end = impl_->current_url_.find('/', scheme_end + 3);
                        std::string base = (host_end != std::string::npos)
                            ? impl_->current_url_.substr(0, host_end)
                            : impl_->current_url_;
                        link.url = base + link.url;
                    }
                } else if (link.url.find("://") == std::string::npos &&
                           !link.url.empty() && link.url[0] != '#') {
                    // 相对路径
                    size_t last_slash = impl_->current_url_.rfind('/');
                    if (last_slash != std::string::npos) {
                        std::string base = impl_->current_url_.substr(0, last_slash + 1);
                        link.url = base + link.url;
                    }
                }
            }
        }
    }

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
