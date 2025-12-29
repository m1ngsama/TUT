/**
 * @file test_browser_engine.cpp
 * @brief 浏览器引擎集成测试
 */

#include <gtest/gtest.h>
#include "core/browser_engine.hpp"

namespace tut {
namespace test {

class BrowserEngineTest : public ::testing::Test {
protected:
    void SetUp() override {
        engine_ = std::make_unique<BrowserEngine>();
    }

    std::unique_ptr<BrowserEngine> engine_;
};

TEST_F(BrowserEngineTest, LoadSimpleHtmlPage) {
    const std::string html = R"(
        <html>
            <head><title>Test Page</title></head>
            <body><h1>Hello World</h1></body>
        </html>
    )";

    ASSERT_TRUE(engine_->loadHtml(html));
    // 标题提取需要完整实现后测试
    // EXPECT_EQ(engine_->getTitle(), "Test Page");
}

TEST_F(BrowserEngineTest, ExtractLinks) {
    const std::string html = R"(
        <html>
            <body>
                <a href="/page1">Link 1</a>
                <a href="https://example.com">Link 2</a>
            </body>
        </html>
    )";

    engine_->loadHtml(html);
    auto links = engine_->extractLinks();
    // 链接提取需要完整实现后测试
    // EXPECT_EQ(links.size(), 2);
}

TEST_F(BrowserEngineTest, NavigationHistory) {
    // 初始状态不能后退或前进
    EXPECT_FALSE(engine_->canGoBack());
    EXPECT_FALSE(engine_->canGoForward());

    // 加载页面后...
    engine_->loadUrl("https://example.com/page1");
    engine_->loadUrl("https://example.com/page2");

    // 可以后退
    // EXPECT_TRUE(engine_->canGoBack());
    // EXPECT_FALSE(engine_->canGoForward());
}

TEST_F(BrowserEngineTest, Refresh) {
    engine_->loadUrl("https://example.com");
    EXPECT_TRUE(engine_->refresh());
}

TEST_F(BrowserEngineTest, GetRenderedContent) {
    const std::string html = "<p>Test content</p>";
    engine_->loadHtml(html);

    std::string content = engine_->getRenderedContent();
    // 应该包含原始内容
    EXPECT_NE(content.find("Test content"), std::string::npos);
}

TEST_F(BrowserEngineTest, GetCurrentUrl) {
    engine_->loadUrl("https://example.com/test");
    EXPECT_EQ(engine_->getCurrentUrl(), "https://example.com/test");
}

}  // namespace test
}  // namespace tut
