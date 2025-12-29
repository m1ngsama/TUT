/**
 * @file http_client.hpp
 * @brief HTTP 客户端模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <map>
#include <optional>
#include <functional>
#include <memory>

namespace tut {

/**
 * @brief HTTP 响应结构体
 */
struct HttpResponse {
    int status_code{0};                              ///< HTTP 状态码
    std::string status_text;                         ///< 状态文本
    std::map<std::string, std::string> headers;      ///< 响应头
    std::string body;                                ///< 响应体
    std::string error;                               ///< 错误信息
    double elapsed_time{0.0};                        ///< 请求耗时(秒)

    bool isSuccess() const { return status_code >= 200 && status_code < 300; }
    bool isRedirect() const { return status_code >= 300 && status_code < 400; }
    bool isError() const { return status_code >= 400 || !error.empty(); }
};

/**
 * @brief HTTP 请求配置
 */
struct HttpConfig {
    int timeout_seconds{30};           ///< 超时时间(秒)
    int max_redirects{5};              ///< 最大重定向次数
    bool follow_redirects{true};       ///< 是否自动跟随重定向
    bool verify_ssl{true};             ///< 是否验证 SSL 证书
    std::string user_agent{"TUT/0.1"}; ///< User-Agent 字符串
};

/**
 * @brief 下载进度回调函数类型
 * @param downloaded 已下载字节数
 * @param total 总字节数 (如果未知则为 0)
 */
using ProgressCallback = std::function<void(size_t downloaded, size_t total)>;

/**
 * @brief HTTP 客户端类
 *
 * 负责发送 HTTP/HTTPS 请求
 *
 * @example
 * @code
 * HttpClient client;
 * auto response = client.get("https://example.com");
 * if (response.isSuccess()) {
 *     std::cout << response.body << std::endl;
 * }
 * @endcode
 */
class HttpClient {
public:
    /**
     * @brief 构造函数
     * @param config HTTP 配置
     */
    explicit HttpClient(const HttpConfig& config = HttpConfig{});

    /**
     * @brief 析构函数
     */
    ~HttpClient();

    /**
     * @brief 发送 GET 请求
     * @param url 请求 URL
     * @param headers 额外的请求头
     * @return HTTP 响应
     */
    HttpResponse get(const std::string& url,
                     const std::map<std::string, std::string>& headers = {});

    /**
     * @brief 发送 POST 请求
     * @param url 请求 URL
     * @param body 请求体
     * @param content_type Content-Type
     * @param headers 额外的请求头
     * @return HTTP 响应
     */
    HttpResponse post(const std::string& url,
                      const std::string& body,
                      const std::string& content_type = "application/x-www-form-urlencoded",
                      const std::map<std::string, std::string>& headers = {});

    /**
     * @brief 发送 HEAD 请求
     * @param url 请求 URL
     * @return HTTP 响应
     */
    HttpResponse head(const std::string& url);

    /**
     * @brief 下载文件
     * @param url 文件 URL
     * @param filepath 保存路径
     * @param progress 进度回调
     * @return 下载成功返回 true
     */
    bool download(const std::string& url,
                  const std::string& filepath,
                  ProgressCallback progress = nullptr);

    /**
     * @brief 设置配置
     * @param config 新配置
     */
    void setConfig(const HttpConfig& config);

    /**
     * @brief 获取当前配置
     * @return 当前配置
     */
    const HttpConfig& getConfig() const;

    /**
     * @brief 设置 Cookie
     * @param domain 域名
     * @param name Cookie 名
     * @param value Cookie 值
     */
    void setCookie(const std::string& domain,
                   const std::string& name,
                   const std::string& value);

    /**
     * @brief 获取 Cookie
     * @param domain 域名
     * @param name Cookie 名
     * @return Cookie 值
     */
    std::optional<std::string> getCookie(const std::string& domain,
                                          const std::string& name) const;

    /**
     * @brief 清除所有 Cookie
     */
    void clearCookies();

private:
    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
