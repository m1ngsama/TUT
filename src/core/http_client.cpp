/**
 * @file http_client.cpp
 * @brief HTTP 客户端实现
 */

#include "core/http_client.hpp"

// cpp-httplib HTTPS 支持已通过 CMake 启用
#include <httplib.h>

#include <chrono>

namespace tut {

class HttpClient::Impl {
public:
    HttpConfig config;
    std::map<std::string, std::map<std::string, std::string>> cookies;

    HttpResponse makeRequest(const std::string& url,
                             const std::string& method,
                             const std::string& body = "",
                             const std::string& content_type = "",
                             const std::map<std::string, std::string>& extra_headers = {}) {
        HttpResponse response;

        auto start_time = std::chrono::steady_clock::now();

        try {
            // 解析 URL
            size_t scheme_end = url.find("://");
            if (scheme_end == std::string::npos) {
                response.error = "Invalid URL: missing scheme";
                return response;
            }

            std::string scheme = url.substr(0, scheme_end);
            std::string rest = url.substr(scheme_end + 3);

            size_t path_start = rest.find('/');
            std::string host_port = (path_start != std::string::npos)
                                    ? rest.substr(0, path_start)
                                    : rest;
            std::string path = (path_start != std::string::npos)
                               ? rest.substr(path_start)
                               : "/";

            // 创建客户端
            std::unique_ptr<httplib::Client> client;
            if (scheme == "https") {
                client = std::make_unique<httplib::Client>("https://" + host_port);
            } else {
                client = std::make_unique<httplib::Client>("http://" + host_port);
            }

            // 配置客户端
            client->set_connection_timeout(config.timeout_seconds);
            client->set_read_timeout(config.timeout_seconds);
            client->set_follow_location(config.follow_redirects);

            // 设置请求头
            httplib::Headers headers;
            headers.emplace("User-Agent", config.user_agent);
            for (const auto& [key, value] : extra_headers) {
                headers.emplace(key, value);
            }

            // 添加 Cookie
            std::string cookie_str;
            // 提取主机名用于 cookie 查找
            std::string host = host_port;
            size_t colon_pos = host.find(':');
            if (colon_pos != std::string::npos) {
                host = host.substr(0, colon_pos);
            }

            auto domain_cookies = cookies.find(host);
            if (domain_cookies != cookies.end()) {
                for (const auto& [name, value] : domain_cookies->second) {
                    if (!cookie_str.empty()) cookie_str += "; ";
                    cookie_str += name + "=" + value;
                }
                if (!cookie_str.empty()) {
                    headers.emplace("Cookie", cookie_str);
                }
            }

            // 发送请求
            httplib::Result result;
            if (method == "GET") {
                result = client->Get(path, headers);
            } else if (method == "POST") {
                result = client->Post(path, headers, body, content_type);
            } else if (method == "HEAD") {
                result = client->Head(path, headers);
            }

            auto end_time = std::chrono::steady_clock::now();
            response.elapsed_time = std::chrono::duration<double>(end_time - start_time).count();

            if (result) {
                response.status_code = result->status;
                response.body = result->body;

                for (const auto& [key, value] : result->headers) {
                    response.headers[key] = value;
                }
            } else {
                response.error = "Request failed: " + httplib::to_string(result.error());
            }

        } catch (const std::exception& e) {
            response.error = std::string("Exception: ") + e.what();
        }

        return response;
    }
};

HttpClient::HttpClient(const HttpConfig& config)
    : impl_(std::make_unique<Impl>()) {
    impl_->config = config;
}

HttpClient::~HttpClient() = default;

HttpResponse HttpClient::get(const std::string& url,
                              const std::map<std::string, std::string>& headers) {
    return impl_->makeRequest(url, "GET", "", "", headers);
}

HttpResponse HttpClient::post(const std::string& url,
                               const std::string& body,
                               const std::string& content_type,
                               const std::map<std::string, std::string>& headers) {
    return impl_->makeRequest(url, "POST", body, content_type, headers);
}

HttpResponse HttpClient::head(const std::string& url) {
    return impl_->makeRequest(url, "HEAD");
}

bool HttpClient::download(const std::string& url,
                          const std::string& filepath,
                          ProgressCallback progress) {
    // TODO: 实现文件下载
    (void)url;
    (void)filepath;
    (void)progress;
    return false;
}

void HttpClient::setConfig(const HttpConfig& config) {
    impl_->config = config;
}

const HttpConfig& HttpClient::getConfig() const {
    return impl_->config;
}

void HttpClient::setCookie(const std::string& domain,
                           const std::string& name,
                           const std::string& value) {
    impl_->cookies[domain][name] = value;
}

std::optional<std::string> HttpClient::getCookie(const std::string& domain,
                                                  const std::string& name) const {
    auto domain_it = impl_->cookies.find(domain);
    if (domain_it == impl_->cookies.end()) return std::nullopt;

    auto cookie_it = domain_it->second.find(name);
    if (cookie_it == domain_it->second.end()) return std::nullopt;

    return cookie_it->second;
}

void HttpClient::clearCookies() {
    impl_->cookies.clear();
}

}  // namespace tut
