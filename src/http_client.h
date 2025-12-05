#pragma once

#include <string>
#include <memory>

struct HttpResponse {
    int status_code;
    std::string body;
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

    // 获取网页内容
    HttpResponse fetch(const std::string& url);

    // 设置超时（秒）
    void set_timeout(long timeout_seconds);

    // 设置用户代理
    void set_user_agent(const std::string& user_agent);

    // 设置是否跟随重定向
    void set_follow_redirects(bool follow);

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
