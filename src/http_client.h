#pragma once

#include <string>
#include <vector>
#include <cstdint>
#include <memory>

// 异步请求状态
enum class AsyncState {
    IDLE,       // 无活跃请求
    LOADING,    // 请求进行中
    COMPLETE,   // 请求成功完成
    FAILED,     // 请求失败
    CANCELLED   // 请求被取消
};

struct HttpResponse {
    int status_code;
    std::string body;
    std::string content_type;
    std::string error_message;

    bool is_success() const {
        return status_code >= 200 && status_code < 300;
    }

    bool is_image() const {
        return content_type.find("image/") == 0;
    }
};

struct BinaryResponse {
    int status_code;
    std::vector<uint8_t> data;
    std::string content_type;
    std::string error_message;

    bool is_success() const {
        return status_code >= 200 && status_code < 300;
    }
};

class HttpClient {
public:
    HttpClient();
    ~HttpClient();

    // 同步请求接口
    HttpResponse fetch(const std::string& url);
    BinaryResponse fetch_binary(const std::string& url);
    HttpResponse post(const std::string& url, const std::string& data,
                     const std::string& content_type = "application/x-www-form-urlencoded");

    // 异步请求接口
    void start_async_fetch(const std::string& url);
    AsyncState poll_async();  // 非阻塞轮询，返回当前状态
    HttpResponse get_async_result();  // 获取结果并重置状态
    void cancel_async();  // 取消当前异步请求
    bool is_async_active() const;  // 是否有活跃的异步请求

    // 配置
    void set_timeout(long timeout_seconds);
    void set_user_agent(const std::string& user_agent);
    void set_follow_redirects(bool follow);
    void enable_cookies(const std::string& cookie_file = "");

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
