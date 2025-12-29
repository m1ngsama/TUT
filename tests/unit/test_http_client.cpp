/**
 * @file test_http_client.cpp
 * @brief HTTP 客户端单元测试
 */

#include <gtest/gtest.h>
#include "core/http_client.hpp"

namespace tut {
namespace test {

class HttpClientTest : public ::testing::Test {
protected:
    HttpClient client_;
};

TEST_F(HttpClientTest, GetRequest) {
    auto response = client_.get("https://httpbin.org/get");
    EXPECT_TRUE(response.isSuccess());
    EXPECT_EQ(response.status_code, 200);
    EXPECT_FALSE(response.body.empty());
}

TEST_F(HttpClientTest, PostRequest) {
    auto response = client_.post(
        "https://httpbin.org/post",
        "key=value",
        "application/x-www-form-urlencoded"
    );
    EXPECT_TRUE(response.isSuccess());
    EXPECT_EQ(response.status_code, 200);
}

TEST_F(HttpClientTest, HeadRequest) {
    auto response = client_.head("https://httpbin.org/get");
    EXPECT_TRUE(response.isSuccess());
    EXPECT_TRUE(response.body.empty());  // HEAD 请求没有 body
}

TEST_F(HttpClientTest, InvalidUrl) {
    auto response = client_.get("https://invalid.invalid.invalid/");
    EXPECT_TRUE(response.isError());
}

TEST_F(HttpClientTest, TimeoutConfig) {
    HttpConfig config;
    config.timeout_seconds = 5;
    client_.setConfig(config);

    EXPECT_EQ(client_.getConfig().timeout_seconds, 5);
}

TEST_F(HttpClientTest, CookieManagement) {
    client_.setCookie("example.com", "session", "abc123");

    auto cookie = client_.getCookie("example.com", "session");
    ASSERT_TRUE(cookie.has_value());
    EXPECT_EQ(*cookie, "abc123");

    client_.clearCookies();
    cookie = client_.getCookie("example.com", "session");
    EXPECT_FALSE(cookie.has_value());
}

TEST_F(HttpClientTest, Redirect) {
    // httpbin.org/redirect/n redirects n times then returns 200
    auto response = client_.get("https://httpbin.org/redirect/1");
    EXPECT_TRUE(response.isSuccess());
    EXPECT_EQ(response.status_code, 200);
}

TEST_F(HttpClientTest, NotFound) {
    auto response = client_.get("https://httpbin.org/status/404");
    EXPECT_FALSE(response.isSuccess());
    EXPECT_EQ(response.status_code, 404);
}

}  // namespace test
}  // namespace tut
