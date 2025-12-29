/**
 * @file address_bar.hpp
 * @brief 地址栏组件
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <vector>
#include <functional>
#include <memory>

namespace tut {

/**
 * @brief 地址栏组件类
 */
class AddressBar {
public:
    AddressBar();
    ~AddressBar();

    /**
     * @brief 设置当前 URL
     */
    void setUrl(const std::string& url);

    /**
     * @brief 获取当前 URL
     */
    std::string getUrl() const;

    /**
     * @brief 设置历史记录 (用于自动补全)
     */
    void setHistory(const std::vector<std::string>& history);

    /**
     * @brief 注册 URL 提交回调
     */
    void onSubmit(std::function<void(const std::string&)> callback);

    /**
     * @brief 聚焦地址栏
     */
    void focus();

    /**
     * @brief 取消聚焦
     */
    void blur();

    /**
     * @brief 是否处于聚焦状态
     */
    bool isFocused() const;

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
