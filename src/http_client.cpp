#include "http_client.h"
#include <curl/curl.h>
#include <stdexcept>

// 回调函数用于接收文本数据
static size_t write_callback(void* contents, size_t size, size_t nmemb, std::string* userp) {
    size_t total_size = size * nmemb;
    userp->append(static_cast<char*>(contents), total_size);
    return total_size;
}

// 回调函数用于接收二进制数据
static size_t binary_write_callback(void* contents, size_t size, size_t nmemb, std::vector<uint8_t>* userp) {
    size_t total_size = size * nmemb;
    uint8_t* data = static_cast<uint8_t*>(contents);
    userp->insert(userp->end(), data, data + total_size);
    return total_size;
}

class HttpClient::Impl {
public:
    CURL* curl;
    long timeout;
    std::string user_agent;
    bool follow_redirects;
    std::string cookie_file;

    // 异步请求相关
    CURLM* multi_handle = nullptr;
    CURL* async_easy = nullptr;
    AsyncState async_state = AsyncState::IDLE;
    std::string async_response_body;
    HttpResponse async_result;

    Impl() : timeout(30),
             user_agent("TUT-Browser/2.0 (Terminal User Interface Browser)"),
             follow_redirects(true) {
        curl = curl_easy_init();
        if (!curl) {
            throw std::runtime_error("Failed to initialize CURL");
        }
        // Enable cookie engine by default (in-memory)
        curl_easy_setopt(curl, CURLOPT_COOKIEFILE, "");
        // Enable automatic decompression of supported encodings (gzip, deflate, etc.)
        curl_easy_setopt(curl, CURLOPT_ACCEPT_ENCODING, "");

        // 初始化multi handle用于异步请求
        multi_handle = curl_multi_init();
        if (!multi_handle) {
            throw std::runtime_error("Failed to initialize CURL multi handle");
        }
    }

    ~Impl() {
        // 清理异步请求
        cleanup_async();

        if (multi_handle) {
            curl_multi_cleanup(multi_handle);
        }
        if (curl) {
            curl_easy_cleanup(curl);
        }
    }

    void cleanup_async() {
        if (async_easy) {
            curl_multi_remove_handle(multi_handle, async_easy);
            curl_easy_cleanup(async_easy);
            async_easy = nullptr;
        }
        async_state = AsyncState::IDLE;
        async_response_body.clear();
    }

    void setup_easy_handle(CURL* handle, const std::string& url) {
        curl_easy_setopt(handle, CURLOPT_URL, url.c_str());
        curl_easy_setopt(handle, CURLOPT_TIMEOUT, timeout);
        curl_easy_setopt(handle, CURLOPT_CONNECTTIMEOUT, 10L);
        curl_easy_setopt(handle, CURLOPT_USERAGENT, user_agent.c_str());

        if (follow_redirects) {
            curl_easy_setopt(handle, CURLOPT_FOLLOWLOCATION, 1L);
            curl_easy_setopt(handle, CURLOPT_MAXREDIRS, 10L);
        }

        curl_easy_setopt(handle, CURLOPT_SSL_VERIFYPEER, 1L);
        curl_easy_setopt(handle, CURLOPT_SSL_VERIFYHOST, 2L);

        if (!cookie_file.empty()) {
            curl_easy_setopt(handle, CURLOPT_COOKIEFILE, cookie_file.c_str());
            curl_easy_setopt(handle, CURLOPT_COOKIEJAR, cookie_file.c_str());
        } else {
            curl_easy_setopt(handle, CURLOPT_COOKIEFILE, "");
        }

        curl_easy_setopt(handle, CURLOPT_ACCEPT_ENCODING, "");
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

BinaryResponse HttpClient::fetch_binary(const std::string& url) {
    BinaryResponse response;
    response.status_code = 0;

    if (!pImpl->curl) {
        response.error_message = "CURL not initialized";
        return response;
    }

    curl_easy_reset(pImpl->curl);

    // 设置URL
    curl_easy_setopt(pImpl->curl, CURLOPT_URL, url.c_str());

    // 设置写回调
    std::vector<uint8_t> response_data;
    curl_easy_setopt(pImpl->curl, CURLOPT_WRITEFUNCTION, binary_write_callback);
    curl_easy_setopt(pImpl->curl, CURLOPT_WRITEDATA, &response_data);

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

    response.data = std::move(response_data);

    return response;
}

HttpResponse HttpClient::post(const std::string& url, const std::string& data,
                              const std::string& content_type) {
    HttpResponse response;
    response.status_code = 0;

    if (!pImpl->curl) {
        response.error_message = "CURL not initialized";
        return response;
    }

    curl_easy_reset(pImpl->curl);

    // Re-apply settings
    curl_easy_setopt(pImpl->curl, CURLOPT_URL, url.c_str());

    // Set write callback
    std::string response_body;
    curl_easy_setopt(pImpl->curl, CURLOPT_WRITEFUNCTION, write_callback);
    curl_easy_setopt(pImpl->curl, CURLOPT_WRITEDATA, &response_body);

    // Set timeout
    curl_easy_setopt(pImpl->curl, CURLOPT_TIMEOUT, pImpl->timeout);
    curl_easy_setopt(pImpl->curl, CURLOPT_CONNECTTIMEOUT, 10L);

    // Set user agent
    curl_easy_setopt(pImpl->curl, CURLOPT_USERAGENT, pImpl->user_agent.c_str());

    // Set redirect following
    if (pImpl->follow_redirects) {
        curl_easy_setopt(pImpl->curl, CURLOPT_FOLLOWLOCATION, 1L);
        curl_easy_setopt(pImpl->curl, CURLOPT_MAXREDIRS, 10L);
    }

    // HTTPS support
    curl_easy_setopt(pImpl->curl, CURLOPT_SSL_VERIFYPEER, 1L);
    curl_easy_setopt(pImpl->curl, CURLOPT_SSL_VERIFYHOST, 2L);

    // Cookie settings
    if (!pImpl->cookie_file.empty()) {
        curl_easy_setopt(pImpl->curl, CURLOPT_COOKIEFILE, pImpl->cookie_file.c_str());
        curl_easy_setopt(pImpl->curl, CURLOPT_COOKIEJAR, pImpl->cookie_file.c_str());
    } else {
        curl_easy_setopt(pImpl->curl, CURLOPT_COOKIEFILE, "");
    }

    // Enable automatic decompression
    curl_easy_setopt(pImpl->curl, CURLOPT_ACCEPT_ENCODING, "");

    // Set POST method
    curl_easy_setopt(pImpl->curl, CURLOPT_POST, 1L);

    // Set POST data
    curl_easy_setopt(pImpl->curl, CURLOPT_POSTFIELDS, data.c_str());
    curl_easy_setopt(pImpl->curl, CURLOPT_POSTFIELDSIZE, data.length());

    // Set Content-Type header
    struct curl_slist* headers = nullptr;
    std::string content_type_header = "Content-Type: " + content_type;
    headers = curl_slist_append(headers, content_type_header.c_str());
    curl_easy_setopt(pImpl->curl, CURLOPT_HTTPHEADER, headers);

    // Perform request
    CURLcode res = curl_easy_perform(pImpl->curl);

    // Clean up headers
    curl_slist_free_all(headers);

    if (res != CURLE_OK) {
        response.error_message = curl_easy_strerror(res);
        return response;
    }

    // Get response code
    long http_code = 0;
    curl_easy_getinfo(pImpl->curl, CURLINFO_RESPONSE_CODE, &http_code);
    response.status_code = static_cast<int>(http_code);

    // Get Content-Type
    char* resp_content_type = nullptr;
    curl_easy_getinfo(pImpl->curl, CURLINFO_CONTENT_TYPE, &resp_content_type);
    if (resp_content_type) {
        response.content_type = resp_content_type;
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

// ==================== 异步请求实现 ====================

void HttpClient::start_async_fetch(const std::string& url) {
    // 如果有正在进行的请求，先取消
    if (pImpl->async_easy) {
        cancel_async();
    }

    // 创建新的easy handle
    pImpl->async_easy = curl_easy_init();
    if (!pImpl->async_easy) {
        pImpl->async_state = AsyncState::FAILED;
        pImpl->async_result.error_message = "Failed to create CURL handle";
        return;
    }

    // 配置请求
    pImpl->setup_easy_handle(pImpl->async_easy, url);

    // 设置写回调
    pImpl->async_response_body.clear();
    curl_easy_setopt(pImpl->async_easy, CURLOPT_WRITEFUNCTION, write_callback);
    curl_easy_setopt(pImpl->async_easy, CURLOPT_WRITEDATA, &pImpl->async_response_body);

    // 添加到multi handle
    curl_multi_add_handle(pImpl->multi_handle, pImpl->async_easy);

    pImpl->async_state = AsyncState::LOADING;
    pImpl->async_result = HttpResponse{};  // 重置结果
}

AsyncState HttpClient::poll_async() {
    if (pImpl->async_state != AsyncState::LOADING) {
        return pImpl->async_state;
    }

    // 执行非阻塞的multi perform
    int still_running = 0;
    CURLMcode mc = curl_multi_perform(pImpl->multi_handle, &still_running);

    if (mc != CURLM_OK) {
        pImpl->async_result.error_message = curl_multi_strerror(mc);
        pImpl->async_state = AsyncState::FAILED;
        pImpl->cleanup_async();
        return pImpl->async_state;
    }

    // 检查是否有完成的请求
    int msgs_left = 0;
    CURLMsg* msg;
    while ((msg = curl_multi_info_read(pImpl->multi_handle, &msgs_left))) {
        if (msg->msg == CURLMSG_DONE) {
            CURL* easy = msg->easy_handle;
            CURLcode result = msg->data.result;

            if (result == CURLE_OK) {
                // 获取响应信息
                long http_code = 0;
                curl_easy_getinfo(easy, CURLINFO_RESPONSE_CODE, &http_code);
                pImpl->async_result.status_code = static_cast<int>(http_code);

                char* content_type = nullptr;
                curl_easy_getinfo(easy, CURLINFO_CONTENT_TYPE, &content_type);
                if (content_type) {
                    pImpl->async_result.content_type = content_type;
                }

                pImpl->async_result.body = std::move(pImpl->async_response_body);
                pImpl->async_state = AsyncState::COMPLETE;
            } else {
                pImpl->async_result.error_message = curl_easy_strerror(result);
                pImpl->async_state = AsyncState::FAILED;
            }

            // 清理handle但保留状态供获取结果
            curl_multi_remove_handle(pImpl->multi_handle, pImpl->async_easy);
            curl_easy_cleanup(pImpl->async_easy);
            pImpl->async_easy = nullptr;
        }
    }

    return pImpl->async_state;
}

HttpResponse HttpClient::get_async_result() {
    HttpResponse result = std::move(pImpl->async_result);
    pImpl->async_result = HttpResponse{};
    pImpl->async_state = AsyncState::IDLE;
    return result;
}

void HttpClient::cancel_async() {
    if (pImpl->async_easy) {
        pImpl->cleanup_async();
        pImpl->async_state = AsyncState::CANCELLED;
    }
}

bool HttpClient::is_async_active() const {
    return pImpl->async_state == AsyncState::LOADING;
}