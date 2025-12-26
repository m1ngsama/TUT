#pragma once

#include <string>
#include <vector>
#include <cstdint>
#include <memory>

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

    HttpResponse fetch(const std::string& url);
    BinaryResponse fetch_binary(const std::string& url);
    HttpResponse post(const std::string& url, const std::string& data,
                     const std::string& content_type = "application/x-www-form-urlencoded");
    void set_timeout(long timeout_seconds);
    void set_user_agent(const std::string& user_agent);
    void set_follow_redirects(bool follow);
    void enable_cookies(const std::string& cookie_file = "");

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
