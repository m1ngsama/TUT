/**
 * @file url_parser.cpp
 * @brief URL 解析器实现
 */

#include "core/url_parser.hpp"
#include <regex>
#include <algorithm>
#include <sstream>
#include <iomanip>

namespace tut {

std::optional<UrlResult> UrlParser::parse(const std::string& url) const {
    if (url.empty()) {
        return std::nullopt;
    }

    // URL 正则表达式
    // 格式: scheme://[userinfo@]host[:port][/path][?query][#fragment]
    static const std::regex url_regex(
        R"(^(([^:/?#]+):)?(//([^/?#]*))?([^?#]*)(\?([^#]*))?(#(.*))?)",
        std::regex::ECMAScript
    );

    std::smatch matches;
    if (!std::regex_match(url, matches, url_regex)) {
        return std::nullopt;
    }

    UrlResult result;

    // 解析 scheme
    if (matches[2].matched) {
        result.scheme = matches[2].str();
        std::transform(result.scheme.begin(), result.scheme.end(),
                       result.scheme.begin(), ::tolower);
        if (!validateScheme(result.scheme)) {
            return std::nullopt;
        }
    }

    // 解析 authority (userinfo@host:port)
    if (matches[4].matched) {
        std::string authority = matches[4].str();

        // 提取 userinfo
        size_t at_pos = authority.find('@');
        if (at_pos != std::string::npos) {
            result.userinfo = authority.substr(0, at_pos);
            authority = authority.substr(at_pos + 1);
        }

        // 提取端口号
        size_t colon_pos = authority.rfind(':');
        if (colon_pos != std::string::npos) {
            std::string port_str = authority.substr(colon_pos + 1);
            try {
                result.port = static_cast<uint16_t>(std::stoi(port_str));
            } catch (...) {
                result.port = getDefaultPort(result.scheme);
            }
            result.host = authority.substr(0, colon_pos);
        } else {
            result.host = authority;
            result.port = getDefaultPort(result.scheme);
        }

        if (!validateHost(result.host)) {
            return std::nullopt;
        }
    }

    // 解析 path
    if (matches[5].matched) {
        result.path = matches[5].str();
        if (result.path.empty()) {
            result.path = "/";
        }
    }

    // 解析 query
    if (matches[7].matched) {
        result.query = matches[7].str();
    }

    // 解析 fragment
    if (matches[9].matched) {
        result.fragment = matches[9].str();
    }

    return result;
}

std::string UrlParser::resolveRelative(const std::string& base,
                                        const std::string& relative) const {
    if (relative.empty()) {
        return base;
    }

    // 如果是绝对 URL，直接返回
    if (relative.find("://") != std::string::npos) {
        return relative;
    }

    auto base_result = parse(base);
    if (!base_result) {
        return relative;
    }

    // 协议相对 URL (//example.com/path)
    if (relative.substr(0, 2) == "//") {
        return base_result->scheme + ":" + relative;
    }

    std::string result = base_result->scheme + "://" + base_result->host;
    if (base_result->port != getDefaultPort(base_result->scheme)) {
        result += ":" + std::to_string(base_result->port);
    }

    // 绝对路径
    if (relative[0] == '/') {
        result += relative;
    } else {
        // 相对路径
        std::string base_path = base_result->path;
        size_t last_slash = base_path.rfind('/');
        if (last_slash != std::string::npos) {
            base_path = base_path.substr(0, last_slash + 1);
        }
        result += base_path + relative;
    }

    return normalize(result);
}

std::string UrlParser::normalize(const std::string& url) const {
    auto result = parse(url);
    if (!result) {
        return url;
    }

    // 移除路径中的 . 和 ..
    std::vector<std::string> segments;
    std::istringstream iss(result->path);
    std::string segment;

    while (std::getline(iss, segment, '/')) {
        if (segment == "..") {
            if (!segments.empty()) {
                segments.pop_back();
            }
        } else if (segment != "." && !segment.empty()) {
            segments.push_back(segment);
        }
    }

    std::string normalized_path = "/";
    for (size_t i = 0; i < segments.size(); ++i) {
        normalized_path += segments[i];
        if (i < segments.size() - 1) {
            normalized_path += "/";
        }
    }

    // 重建 URL
    std::string normalized = result->scheme + "://" + result->host;
    if (result->port != getDefaultPort(result->scheme)) {
        normalized += ":" + std::to_string(result->port);
    }
    normalized += normalized_path;

    if (!result->query.empty()) {
        normalized += "?" + result->query;
    }
    if (!result->fragment.empty()) {
        normalized += "#" + result->fragment;
    }

    return normalized;
}

std::string UrlParser::encode(const std::string& str) {
    std::ostringstream encoded;
    encoded << std::hex << std::uppercase;

    for (unsigned char c : str) {
        if (std::isalnum(c) || c == '-' || c == '_' || c == '.' || c == '~') {
            encoded << c;
        } else {
            encoded << '%' << std::setw(2) << std::setfill('0') << static_cast<int>(c);
        }
    }

    return encoded.str();
}

std::string UrlParser::decode(const std::string& str) {
    std::string decoded;
    decoded.reserve(str.size());

    for (size_t i = 0; i < str.size(); ++i) {
        if (str[i] == '%' && i + 2 < str.size()) {
            int value;
            std::istringstream iss(str.substr(i + 1, 2));
            if (iss >> std::hex >> value) {
                decoded += static_cast<char>(value);
                i += 2;
            } else {
                decoded += str[i];
            }
        } else if (str[i] == '+') {
            decoded += ' ';
        } else {
            decoded += str[i];
        }
    }

    return decoded;
}

bool UrlParser::validateScheme(const std::string& scheme) const {
    return scheme == "http" || scheme == "https" || scheme == "file";
}

bool UrlParser::validateHost(const std::string& host) const {
    return !host.empty();
}

uint16_t UrlParser::getDefaultPort(const std::string& scheme) const {
    if (scheme == "https") return 443;
    if (scheme == "http") return 80;
    return 0;
}

}  // namespace tut
