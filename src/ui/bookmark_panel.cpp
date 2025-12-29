/**
 * @file bookmark_panel.cpp
 * @brief 书签面板组件实现
 */

#include "ui/bookmark_panel.hpp"
#include <algorithm>

namespace tut {

class BookmarkPanel::Impl {
public:
    std::vector<BookmarkItem> bookmarks_;
    int selected_index_{0};
    bool visible_{false};
    std::function<void(const BookmarkItem&)> on_select_;
};

BookmarkPanel::BookmarkPanel() : impl_(std::make_unique<Impl>()) {}

BookmarkPanel::~BookmarkPanel() = default;

void BookmarkPanel::setBookmarks(const std::vector<BookmarkItem>& bookmarks) {
    impl_->bookmarks_ = bookmarks;
    impl_->selected_index_ = bookmarks.empty() ? -1 : 0;
}

std::vector<BookmarkItem> BookmarkPanel::getBookmarks() const {
    return impl_->bookmarks_;
}

void BookmarkPanel::addBookmark(const BookmarkItem& bookmark) {
    impl_->bookmarks_.push_back(bookmark);
}

void BookmarkPanel::removeBookmark(const std::string& id) {
    impl_->bookmarks_.erase(
        std::remove_if(impl_->bookmarks_.begin(), impl_->bookmarks_.end(),
                       [&id](const BookmarkItem& item) { return item.id == id; }),
        impl_->bookmarks_.end());
}

void BookmarkPanel::selectNext() {
    if (impl_->bookmarks_.empty()) return;
    impl_->selected_index_ =
        (impl_->selected_index_ + 1) % static_cast<int>(impl_->bookmarks_.size());
}

void BookmarkPanel::selectPrevious() {
    if (impl_->bookmarks_.empty()) return;
    impl_->selected_index_--;
    if (impl_->selected_index_ < 0) {
        impl_->selected_index_ = static_cast<int>(impl_->bookmarks_.size()) - 1;
    }
}

int BookmarkPanel::getSelectedIndex() const {
    return impl_->selected_index_;
}

void BookmarkPanel::onSelect(std::function<void(const BookmarkItem&)> callback) {
    impl_->on_select_ = std::move(callback);
}

std::vector<BookmarkItem> BookmarkPanel::search(const std::string& query) const {
    std::vector<BookmarkItem> results;
    std::string lower_query = query;
    std::transform(lower_query.begin(), lower_query.end(), lower_query.begin(), ::tolower);

    for (const auto& bookmark : impl_->bookmarks_) {
        std::string lower_title = bookmark.title;
        std::transform(lower_title.begin(), lower_title.end(), lower_title.begin(), ::tolower);

        std::string lower_url = bookmark.url;
        std::transform(lower_url.begin(), lower_url.end(), lower_url.begin(), ::tolower);

        if (lower_title.find(lower_query) != std::string::npos ||
            lower_url.find(lower_query) != std::string::npos) {
            results.push_back(bookmark);
        }
    }
    return results;
}

void BookmarkPanel::setVisible(bool visible) {
    impl_->visible_ = visible;
}

bool BookmarkPanel::isVisible() const {
    return impl_->visible_;
}

}  // namespace tut
