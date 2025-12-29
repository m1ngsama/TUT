/**
 * @file status_bar.cpp
 * @brief 状态栏组件实现
 */

#include "ui/status_bar.hpp"

namespace tut {

class StatusBar::Impl {
public:
    std::string message_;
    LoadingStatus loading_status_;
};

StatusBar::StatusBar() : impl_(std::make_unique<Impl>()) {}

StatusBar::~StatusBar() = default;

void StatusBar::setMessage(const std::string& message) {
    impl_->message_ = message;
}

std::string StatusBar::getMessage() const {
    return impl_->message_;
}

void StatusBar::setLoadingStatus(const LoadingStatus& status) {
    impl_->loading_status_ = status;
}

LoadingStatus StatusBar::getLoadingStatus() const {
    return impl_->loading_status_;
}

void StatusBar::showError(const std::string& error) {
    impl_->message_ = "[ERROR] " + error;
}

void StatusBar::showSuccess(const std::string& message) {
    impl_->message_ = "[OK] " + message;
}

void StatusBar::clear() {
    impl_->message_.clear();
}

}  // namespace tut
