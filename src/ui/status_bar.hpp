/**
 * @file status_bar.hpp
 * @brief 状态栏组件
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <memory>

namespace tut {

/**
 * @brief 加载状态
 */
struct LoadingStatus {
    bool is_loading{false};
    size_t bytes_downloaded{0};
    size_t bytes_total{0};
    double elapsed_time{0.0};
    int link_count{0};
};

/**
 * @brief 状态栏组件类
 */
class StatusBar {
public:
    StatusBar();
    ~StatusBar();

    /**
     * @brief 设置消息
     */
    void setMessage(const std::string& message);

    /**
     * @brief 获取消息
     */
    std::string getMessage() const;

    /**
     * @brief 设置加载状态
     */
    void setLoadingStatus(const LoadingStatus& status);

    /**
     * @brief 获取加载状态
     */
    LoadingStatus getLoadingStatus() const;

    /**
     * @brief 显示错误信息
     */
    void showError(const std::string& error);

    /**
     * @brief 显示成功信息
     */
    void showSuccess(const std::string& message);

    /**
     * @brief 清除消息
     */
    void clear();

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
