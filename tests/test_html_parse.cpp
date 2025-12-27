#include "html_parser.h"
#include "dom_tree.h"
#include <iostream>
#include <cassert>

int main() {
    std::cout << "=== TUT 2.0 HTML Parser Test ===" << std::endl;

    HtmlParser parser;

    // Test 1: Basic HTML parsing
    std::cout << "\n[Test 1] Basic HTML parsing..." << std::endl;
    std::string html1 = R"(
        <!DOCTYPE html>
        <html>
        <head><title>Test Page</title></head>
        <body>
            <h1>Hello World</h1>
            <p>This is a <a href="https://example.com">link</a>.</p>
        </body>
        </html>
    )";

    auto tree1 = parser.parse_tree(html1, "https://test.com");
    std::cout << "  ✓ Title: " << tree1.title << std::endl;
    std::cout << "  ✓ Links found: " << tree1.links.size() << std::endl;

    if (tree1.title == "Test Page" && tree1.links.size() == 1) {
        std::cout << "  ✓ Basic parsing passed" << std::endl;
    } else {
        std::cout << "  ✗ Basic parsing failed" << std::endl;
        return 1;
    }

    // Test 2: Link URL resolution
    std::cout << "\n[Test 2] Link URL resolution..." << std::endl;
    std::string html2 = R"(
        <html>
        <body>
            <a href="/relative">Relative</a>
            <a href="https://absolute.com">Absolute</a>
            <a href="page.html">Same dir</a>
        </body>
        </html>
    )";

    auto tree2 = parser.parse_tree(html2, "https://base.com/dir/");
    std::cout << "  Found " << tree2.links.size() << " links:" << std::endl;
    for (const auto& link : tree2.links) {
        std::cout << "    - " << link.url << std::endl;
    }

    if (tree2.links.size() == 3) {
        std::cout << "  ✓ Link resolution passed" << std::endl;
    } else {
        std::cout << "  ✗ Link resolution failed" << std::endl;
        return 1;
    }

    // Test 3: Form parsing
    std::cout << "\n[Test 3] Form parsing..." << std::endl;
    std::string html3 = R"(
        <html>
        <body>
            <form action="/submit" method="post">
                <input type="text" name="username">
                <input type="password" name="password">
                <button type="submit">Login</button>
            </form>
        </body>
        </html>
    )";

    auto tree3 = parser.parse_tree(html3, "https://form.com");
    std::cout << "  Form fields found: " << tree3.form_fields.size() << std::endl;

    if (tree3.form_fields.size() >= 2) {
        std::cout << "  ✓ Form parsing passed" << std::endl;
    } else {
        std::cout << "  ✗ Form parsing failed" << std::endl;
        return 1;
    }

    // Test 4: Image parsing
    std::cout << "\n[Test 4] Image parsing..." << std::endl;
    std::string html4 = R"(
        <html>
        <body>
            <img src="image1.png" alt="Image 1">
            <img src="/images/image2.jpg" alt="Image 2">
        </body>
        </html>
    )";

    auto tree4 = parser.parse_tree(html4, "https://images.com/page/");
    std::cout << "  Images found: " << tree4.images.size() << std::endl;

    if (tree4.images.size() == 2) {
        std::cout << "  ✓ Image parsing passed" << std::endl;
    } else {
        std::cout << "  ✗ Image parsing failed" << std::endl;
        return 1;
    }

    // Test 5: Unicode content
    std::cout << "\n[Test 5] Unicode content..." << std::endl;
    std::string html5 = R"(
        <html>
        <head><title>中文标题</title></head>
        <body>
            <h1>日本語テスト</h1>
            <p>한국어 테스트</p>
        </body>
        </html>
    )";

    auto tree5 = parser.parse_tree(html5, "https://unicode.com");
    std::cout << "  ✓ Title: " << tree5.title << std::endl;

    if (tree5.title == "中文标题") {
        std::cout << "  ✓ Unicode parsing passed" << std::endl;
    } else {
        std::cout << "  ✗ Unicode parsing failed" << std::endl;
        return 1;
    }

    std::cout << "\n=== All HTML parser tests passed! ===" << std::endl;
    return 0;
}
