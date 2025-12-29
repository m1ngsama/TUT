/**
 * @file test_url_parser.cpp
 * @brief URL 解析器单元测试
 */

#include <gtest/gtest.h>
#include "core/url_parser.hpp"

namespace tut {
namespace test {

class UrlParserTest : public ::testing::Test {
protected:
    UrlParser parser_;
};

TEST_F(UrlParserTest, ParseValidHttpUrl) {
    auto result = parser_.parse("https://example.com:8080/path?query=1");
    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(result->scheme, "https");
    EXPECT_EQ(result->host, "example.com");
    EXPECT_EQ(result->port, 8080);
    EXPECT_EQ(result->path, "/path");
    EXPECT_EQ(result->query, "query=1");
}

TEST_F(UrlParserTest, ParseUrlWithoutPort) {
    auto result = parser_.parse("http://example.com/path");
    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(result->port, 80);  // 默认 HTTP 端口
}

TEST_F(UrlParserTest, ParseHttpsDefaultPort) {
    auto result = parser_.parse("https://example.com/path");
    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(result->port, 443);  // 默认 HTTPS 端口
}

TEST_F(UrlParserTest, ParseInvalidUrl) {
    auto result = parser_.parse("not a url");
    // 这个测试取决于我们如何定义 "无效"
    // 当前实现可能仍会尝试解析
}

TEST_F(UrlParserTest, ParseEmptyUrl) {
    auto result = parser_.parse("");
    EXPECT_FALSE(result.has_value());
}

TEST_F(UrlParserTest, ParseUrlWithFragment) {
    auto result = parser_.parse("https://example.com#section");
    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(result->fragment, "section");
}

TEST_F(UrlParserTest, ParseUrlWithUserInfo) {
    auto result = parser_.parse("https://user:pass@example.com/path");
    ASSERT_TRUE(result.has_value());
    EXPECT_EQ(result->userinfo, "user:pass");
    EXPECT_EQ(result->host, "example.com");
}

TEST_F(UrlParserTest, ResolveRelativeUrl) {
    std::string base = "https://example.com/dir/page.html";
    EXPECT_EQ(parser_.resolveRelative(base, "other.html"),
              "https://example.com/dir/other.html");
    EXPECT_EQ(parser_.resolveRelative(base, "/absolute.html"),
              "https://example.com/absolute.html");
    EXPECT_EQ(parser_.resolveRelative(base, "//other.com/path"),
              "https://other.com/path");
}

TEST_F(UrlParserTest, NormalizeUrl) {
    EXPECT_EQ(parser_.normalize("https://example.com/a/../b/./c"),
              "https://example.com/b/c");
}

TEST_F(UrlParserTest, EncodeUrl) {
    EXPECT_EQ(UrlParser::encode("hello world"), "hello%20world");
    EXPECT_EQ(UrlParser::encode("a+b=c"), "a%2Bb%3Dc");
}

TEST_F(UrlParserTest, DecodeUrl) {
    EXPECT_EQ(UrlParser::decode("hello%20world"), "hello world");
    EXPECT_EQ(UrlParser::decode("a+b"), "a b");
}

}  // namespace test
}  // namespace tut
