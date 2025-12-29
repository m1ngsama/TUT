# tests/CMakeLists.txt
# TUT 测试配置

# ============================================================================
# 单元测试
# ============================================================================

# URL 解析器测试
add_executable(test_url_parser
    unit/test_url_parser.cpp
)
target_link_libraries(test_url_parser PRIVATE
    tut_lib
    GTest::gtest_main
)
add_test(NAME UrlParserTest COMMAND test_url_parser)

# HTTP 客户端测试
add_executable(test_http_client
    unit/test_http_client.cpp
)
target_link_libraries(test_http_client PRIVATE
    tut_lib
    GTest::gtest_main
)
add_test(NAME HttpClientTest COMMAND test_http_client)

# HTML 渲染器测试
add_executable(test_html_renderer
    unit/test_html_renderer.cpp
)
target_link_libraries(test_html_renderer PRIVATE
    tut_lib
    GTest::gtest_main
)
add_test(NAME HtmlRendererTest COMMAND test_html_renderer)

# ============================================================================
# 集成测试
# ============================================================================

add_executable(test_browser_engine
    integration/test_browser_engine.cpp
)
target_link_libraries(test_browser_engine PRIVATE
    tut_lib
    GTest::gtest_main
)
add_test(NAME BrowserEngineTest COMMAND test_browser_engine)

# ============================================================================
# 测试发现
# ============================================================================
include(GoogleTest)
gtest_discover_tests(test_url_parser)
gtest_discover_tests(test_http_client)
gtest_discover_tests(test_html_renderer)
gtest_discover_tests(test_browser_engine)
