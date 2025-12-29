/**
 * @file url_parser.hpp
 * @brief URL 解析器模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <optional>
#include <cstdint>

namespace tut {

/**
 * @brief URL 解析结果结构体
 *
 * 包含解析后的 URL 各个组成部分
 */
struct UrlResult {
    std::string scheme;      ///< 协议 (http, https, file)
    std::string host;        ///< 主机名 (example.com)
    uint16_t port{80};       ///< 端口号
    std::string path;        ///< 路径 (/path/to/resource)
    std::string query;       ///< 查询参数 (key=value&foo=bar)
    std::string fragment;    ///< 片段标识符 (section)
    std::string userinfo;    ///< 用户信息 (user:pass)
};

/**
 * @brief URL 解析器类
 *
 * 负责将 URL 字符串解析为结构化对象
 *
 * @example
 * @code
 * UrlParser parser;
 * auto result = parser.parse("https://example.com:8080/path?query=1");
 * if (result) {
 *     std::cout << "Host: " << result->host << std::endl;
 * }
 * @endcode
 */
class UrlParser {
public:
    /**
     * @brief 默认构造函数
     */
    UrlParser() = default;

    /**
     * @brief 解析 URL 字符串
     *
     * @param url 完整的 URL 字符串
     * @return 解析成功返回 UrlResult，失败返回 std::nullopt
     *
     * @note 支持 http, https, file 协议
     * @note 会自动处理端口号默认值
     */
    std::optional<UrlResult> parse(const std::string& url) const;

    /**
     * @brief 解析相对 URL
     *
     * @param base 基础 URL
     * @param relative 相对 URL
     * @return 解析成功返回完整 URL，失败返回空字符串
     */
    std::string resolveRelative(const std::string& base,
                                const std::string& relative) const;

    /**
     * @brief 规范化 URL
     *
     * @param url 待规范化的 URL
     * @return 规范化后的 URL
     */
    std::string normalize(const std::string& url) const;

    /**
     * @brief URL 编码
     *
     * @param str 待编码的字符串
     * @return 编码后的字符串
     */
    static std::string encode(const std::string& str);

    /**
     * @brief URL 解码
     *
     * @param str 待解码的字符串
     * @return 解码后的字符串
     */
    static std::string decode(const std::string& str);

private:
    bool validateScheme(const std::string& scheme) const;
    bool validateHost(const std::string& host) const;
    uint16_t getDefaultPort(const std::string& scheme) const;
};

}  // namespace tut
