/**
 * @file content_view.hpp
 * @brief 内容视图组件
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <vector>
#include <functional>
#include <memory>
#include "../core/types.hpp"

namespace tut {

/**
 * @brief 内容视图组件类
 *
 * 负责显示渲染后的网页内容
 */
class ContentView {
public:
    ContentView();
    ~ContentView();

    /**
     * @brief 设置内容
     */
    void setContent(const std::string& content);

    /**
     * @brief 设置链接列表
     */
    void setLinks(const std::vector<LinkInfo>& links);

    /**
     * @brief 向下滚动
     */
    void scrollDown(int lines = 1);

    /**
     * @brief 向上滚动
     */
    void scrollUp(int lines = 1);

    /**
     * @brief 滚动到顶部
     */
    void scrollToTop();

    /**
     * @brief 滚动到底部
     */
    void scrollToBottom();

    /**
     * @brief 向下翻页
     */
    void pageDown();

    /**
     * @brief 向上翻页
     */
    void pageUp();

    /**
     * @brief 获取当前滚动位置
     */
    int getScrollPosition() const;

    /**
     * @brief 选择下一个链接
     */
    void selectNextLink();

    /**
     * @brief 选择上一个链接
     */
    void selectPreviousLink();

    /**
     * @brief 获取选中的链接索引
     */
    int getSelectedLinkIndex() const;

    /**
     * @brief 注册链接点击回调
     */
    void onLinkActivate(std::function<void(const std::string&)> callback);

    /**
     * @brief 搜索文本
     * @return 找到的结果数量
     */
    int search(const std::string& query);

    /**
     * @brief 跳转到下一个搜索结果
     */
    void nextSearchResult();

    /**
     * @brief 跳转到上一个搜索结果
     */
    void previousSearchResult();

    /**
     * @brief 清除搜索
     */
    void clearSearch();

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
