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

    HttpResponse fetch(const std::string& url);
    void set_timeout(long timeout_seconds);
    void set_user_agent(const std::string& user_agent);
    void set_follow_redirects(bool follow);

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
