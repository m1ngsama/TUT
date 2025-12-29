/**
 * @file address_bar.cpp
 * @brief 地址栏组件实现
 */

#include "ui/address_bar.hpp"

namespace tut {

class AddressBar::Impl {
public:
    std::string url_;
    std::vector<std::string> history_;
    std::function<void(const std::string&)> on_submit_;
    bool focused_{false};
};

AddressBar::AddressBar() : impl_(std::make_unique<Impl>()) {}

AddressBar::~AddressBar() = default;

void AddressBar::setUrl(const std::string& url) {
    impl_->url_ = url;
}

std::string AddressBar::getUrl() const {
    return impl_->url_;
}

void AddressBar::setHistory(const std::vector<std::string>& history) {
    impl_->history_ = history;
}

void AddressBar::onSubmit(std::function<void(const std::string&)> callback) {
    impl_->on_submit_ = std::move(callback);
}

void AddressBar::focus() {
    impl_->focused_ = true;
}

void AddressBar::blur() {
    impl_->focused_ = false;
}

bool AddressBar::isFocused() const {
    return impl_->focused_;
}

}  // namespace tut
