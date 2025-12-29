/**
 * @file test_html_renderer.cpp
 * @brief HTML 渲染器单元测试
 */

#include <gtest/gtest.h>
#include "renderer/html_renderer.hpp"

namespace tut {
namespace test {

class HtmlRendererTest : public ::testing::Test {
protected:
    HtmlRenderer renderer_;
};

TEST_F(HtmlRendererTest, ExtractTitle) {
    const std::string html = R"(
        <html>
            <head><title>Test Page</title></head>
            <body><h1>Hello</h1></body>
        </html>
    )";

    EXPECT_EQ(renderer_.extractTitle(html), "Test Page");
}

TEST_F(HtmlRendererTest, ExtractTitleMissing) {
    const std::string html = "<html><body>No title</body></html>";
    EXPECT_EQ(renderer_.extractTitle(html), "");
}

TEST_F(HtmlRendererTest, RenderSimpleParagraph) {
    const std::string html = "<p>Hello World</p>";
    auto result = renderer_.render(html);
    EXPECT_FALSE(result.text.empty());
    EXPECT_NE(result.text.find("Hello World"), std::string::npos);
}

TEST_F(HtmlRendererTest, ExtractLinks) {
    const std::string html = R"(
        <html>
            <body>
                <a href="https://example.com">Link 1</a>
                <a href="/relative">Link 2</a>
            </body>
        </html>
    )";

    auto links = renderer_.extractLinks(html);
    EXPECT_EQ(links.size(), 2);
}

TEST_F(HtmlRendererTest, ResolveRelativeLinks) {
    const std::string html = R"(
        <html>
            <body>
                <a href="/page.html">Link</a>
            </body>
        </html>
    )";

    auto links = renderer_.extractLinks(html, "https://example.com/dir/");
    ASSERT_EQ(links.size(), 1);
    EXPECT_EQ(links[0].url, "https://example.com/page.html");
}

TEST_F(HtmlRendererTest, SkipScriptAndStyle) {
    const std::string html = R"(
        <html>
            <head>
                <style>body { color: red; }</style>
                <script>alert('hello');</script>
            </head>
            <body>
                <p>Visible content</p>
            </body>
        </html>
    )";

    auto result = renderer_.render(html);
    EXPECT_NE(result.text.find("Visible content"), std::string::npos);
    EXPECT_EQ(result.text.find("alert"), std::string::npos);
    EXPECT_EQ(result.text.find("color: red"), std::string::npos);
}

TEST_F(HtmlRendererTest, RenderHeadings) {
    const std::string html = R"(
        <h1>Heading 1</h1>
        <h2>Heading 2</h2>
        <p>Paragraph</p>
    )";

    auto result = renderer_.render(html);
    EXPECT_NE(result.text.find("Heading 1"), std::string::npos);
    EXPECT_NE(result.text.find("Heading 2"), std::string::npos);
}

TEST_F(HtmlRendererTest, RenderList) {
    const std::string html = R"(
        <ul>
            <li>Item 1</li>
            <li>Item 2</li>
        </ul>
    )";

    auto result = renderer_.render(html);
    EXPECT_NE(result.text.find("Item 1"), std::string::npos);
    EXPECT_NE(result.text.find("Item 2"), std::string::npos);
}

TEST_F(HtmlRendererTest, RenderWithoutColors) {
    const std::string html = "<a href=\"#\">Link</a>";

    RenderOptions options;
    options.use_colors = false;

    auto result = renderer_.render(html, options);
    // 不应该包含 ANSI 转义码
    EXPECT_EQ(result.text.find("\033["), std::string::npos);
}

}  // namespace test
}  // namespace tut
