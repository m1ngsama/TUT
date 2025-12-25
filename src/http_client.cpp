#include "http_client.h"
#include <curl/curl.h>
#include <stdexcept>

// 回调函数用于接收数据
static size_t write_callback(void* contents, size_t size, size_t nmemb, std::string* userp) {
    size_t total_size = size * nmemb;
    userp->append(static_cast<char*>(contents), total_size);
    return total_size;
}

class HttpClient::Impl {
public:
    CURL* curl;
    long timeout;
    std::string user_agent;
    bool follow_redirects;
    std::string cookie_file;

    Impl() : timeout(30),
             user_agent("TUT-Browser/1.0 (Terminal User Interface Browser)"),
             follow_redirects(true) {
        curl = curl_easy_init();
        if (!curl) {
            throw std::runtime_error("Failed to initialize CURL");
        }
        // Enable cookie engine by default (in-memory)
        curl_easy_setopt(curl, CURLOPT_COOKIEFILE, "");
        // Enable automatic decompression of supported encodings (gzip, deflate, etc.)
        curl_easy_setopt(curl, CURLOPT_ACCEPT_ENCODING, "");
    }

    ~Impl() {
        if (curl) {
            curl_easy_cleanup(curl);
        }
    }
};

HttpClient::HttpClient() : pImpl(std::make_unique<Impl>()) {}

HttpClient::~HttpClient() = default;

HttpResponse HttpClient::fetch(const std::string& url) {
    HttpResponse response;
    response.status_code = 0;

    if (!pImpl->curl) {
        response.error_message = "CURL not initialized";
        return response;
    }

    // 重置选项 (Note: curl_easy_reset clears cookies setting if not careful, 
    // but here we might want to preserve them or reset and re-apply options)
    // Actually curl_easy_reset clears ALL options including cookie engine state?
    // No, it resets options to default. It does NOT clear the cookie engine state (cookies held in memory).
    // BUT it resets CURLOPT_COOKIEFILE/JAR settings.
    
    curl_easy_reset(pImpl->curl);

    // Re-apply settings
    // 设置URL
    curl_easy_setopt(pImpl->curl, CURLOPT_URL, url.c_str());

    // 设置写回调
    std::string response_body;
    curl_easy_setopt(pImpl->curl, CURLOPT_WRITEFUNCTION, write_callback);
    curl_easy_setopt(pImpl->curl, CURLOPT_WRITEDATA, &response_body);

    // 设置超时
    curl_easy_setopt(pImpl->curl, CURLOPT_TIMEOUT, pImpl->timeout);
    curl_easy_setopt(pImpl->curl, CURLOPT_CONNECTTIMEOUT, 10L);

    // 设置用户代理
    curl_easy_setopt(pImpl->curl, CURLOPT_USERAGENT, pImpl->user_agent.c_str());

    // 设置是否跟随重定向
    if (pImpl->follow_redirects) {
        curl_easy_setopt(pImpl->curl, CURLOPT_FOLLOWLOCATION, 1L);
        curl_easy_setopt(pImpl->curl, CURLOPT_MAXREDIRS, 10L);
    }

    // 支持 HTTPS
    curl_easy_setopt(pImpl->curl, CURLOPT_SSL_VERIFYPEER, 1L);
    curl_easy_setopt(pImpl->curl, CURLOPT_SSL_VERIFYHOST, 2L);

    // Cookie settings
    if (!pImpl->cookie_file.empty()) {
        curl_easy_setopt(pImpl->curl, CURLOPT_COOKIEFILE, pImpl->cookie_file.c_str());
        curl_easy_setopt(pImpl->curl, CURLOPT_COOKIEJAR, pImpl->cookie_file.c_str());
    } else {
        curl_easy_setopt(pImpl->curl, CURLOPT_COOKIEFILE, "");
    }

    // 执行请求
    CURLcode res = curl_easy_perform(pImpl->curl);

    if (res != CURLE_OK) {
        response.error_message = curl_easy_strerror(res);
        return response;
    }

    // 获取响应码
    long http_code = 0;
    curl_easy_getinfo(pImpl->curl, CURLINFO_RESPONSE_CODE, &http_code);
    response.status_code = static_cast<int>(http_code);

    // 获取 Content-Type
    char* content_type = nullptr;
    curl_easy_getinfo(pImpl->curl, CURLINFO_CONTENT_TYPE, &content_type);
    if (content_type) {
        response.content_type = content_type;
    }

    response.body = std::move(response_body);

    return response;
}

void HttpClient::set_timeout(long timeout_seconds) {
    pImpl->timeout = timeout_seconds;
}

void HttpClient::set_user_agent(const std::string& user_agent) {
    pImpl->user_agent = user_agent;
}

void HttpClient::set_follow_redirects(bool follow) {
    pImpl->follow_redirects = follow;
}

void HttpClient::enable_cookies(const std::string& cookie_file) {
    pImpl->cookie_file = cookie_file;
}