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

// 异步图片下载任务
struct ImageDownloadTask {
    std::string url;
    void* user_data;  // 用户自定义数据 (例如 DomNode*)
    std::vector<uint8_t> data;
    std::string content_type;
    int status_code = 0;
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

    // 异步请求接口 (页面)
    void start_async_fetch(const std::string& url);
    AsyncState poll_async();  // 非阻塞轮询，返回当前状态
    HttpResponse get_async_result();  // 获取结果并重置状态
    void cancel_async();  // 取消当前异步请求
    bool is_async_active() const;  // 是否有活跃的异步请求

    // 异步图片下载接口 (支持多并发)
    void add_image_download(const std::string& url, void* user_data = nullptr);
    void poll_image_downloads();  // 非阻塞轮询所有图片下载
    std::vector<ImageDownloadTask> get_completed_images();  // 获取并移除已完成的图片
    void cancel_all_images();  // 取消所有图片下载
    int get_pending_image_count() const;  // 获取待下载图片数量
    int get_loading_image_count() const;  // 获取正在下载的图片数量
    void set_max_concurrent_images(int max);  // 设置最大并发数 (默认3)

    // 配置
    void set_timeout(long timeout_seconds);
    void set_user_agent(const std::string& user_agent);
    void set_follow_redirects(bool follow);
    void enable_cookies(const std::string& cookie_file = "");

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
