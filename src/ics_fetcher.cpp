#include "ics_fetcher.h"

#include <curl/curl.h>
#include <stdexcept>
#include <string>

namespace {
size_t write_callback(char *ptr, size_t size, size_t nmemb, void *userdata) {
    auto *buffer = static_cast<std::string *>(userdata);
    buffer->append(ptr, size * nmemb);
    return size * nmemb;
}
} // namespace

std::string fetch_ics(const std::string &url) {
    CURL *curl = curl_easy_init();
    if (!curl) {
        throw std::runtime_error("初始化 libcurl 失败");
    }

    std::string response;
    curl_easy_setopt(curl, CURLOPT_URL, url.c_str());
    curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, 1L);
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, write_callback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, &response);
    curl_easy_setopt(curl, CURLOPT_USERAGENT, "nbtca_tui/1.0");

    CURLcode res = curl_easy_perform(curl);
    if (res != CURLE_OK) {
        std::string err = curl_easy_strerror(res);
        curl_easy_cleanup(curl);
        throw std::runtime_error("请求 ICS 失败: " + err);
    }

    long http_code = 0;
    curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &http_code);
    curl_easy_cleanup(curl);

    if (http_code < 200 || http_code >= 300) {
        throw std::runtime_error("HTTP 状态码错误: " + std::to_string(http_code));
    }

    return response;
}


