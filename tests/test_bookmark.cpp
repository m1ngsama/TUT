#include "bookmark.h"
#include <iostream>
#include <cstdio>

int main() {
    std::cout << "=== TUT 2.0 Bookmark Test ===" << std::endl;

    // Note: Uses default path ~/.config/tut/bookmarks.json
    // We'll test in-memory operations and clean up

    tut::BookmarkManager manager;

    // Store original count to restore later
    size_t original_count = manager.count();
    std::cout << "  Original bookmark count: " << original_count << std::endl;

    // Test 1: Add bookmarks
    std::cout << "\n[Test 1] Add bookmarks..." << std::endl;

    // Use unique URLs to avoid conflicts with existing bookmarks
    std::string test_url1 = "https://test-example-12345.com";
    std::string test_url2 = "https://test-google-12345.com";
    std::string test_url3 = "https://test-github-12345.com";

    bool added1 = manager.add(test_url1, "Test Example");
    bool added2 = manager.add(test_url2, "Test Google");
    bool added3 = manager.add(test_url3, "Test GitHub");

    if (added1 && added2 && added3) {
        std::cout << "  ✓ Added 3 bookmarks" << std::endl;
    } else {
        std::cout << "  ✗ Failed to add bookmarks" << std::endl;
        return 1;
    }

    // Test 2: Duplicate detection
    std::cout << "\n[Test 2] Duplicate detection..." << std::endl;

    bool duplicate = manager.add(test_url1, "Duplicate");
    if (!duplicate) {
        std::cout << "  ✓ Duplicate correctly rejected" << std::endl;
    } else {
        std::cout << "  ✗ Duplicate was incorrectly added" << std::endl;
        // Clean up and fail
        manager.remove(test_url1);
        manager.remove(test_url2);
        manager.remove(test_url3);
        return 1;
    }

    // Test 3: Check existence
    std::cout << "\n[Test 3] Check existence..." << std::endl;

    if (manager.contains(test_url1) && !manager.contains("https://notexist-12345.com")) {
        std::cout << "  ✓ Existence check passed" << std::endl;
    } else {
        std::cout << "  ✗ Existence check failed" << std::endl;
        manager.remove(test_url1);
        manager.remove(test_url2);
        manager.remove(test_url3);
        return 1;
    }

    // Test 4: Count check
    std::cout << "\n[Test 4] Count check..." << std::endl;

    if (manager.count() == original_count + 3) {
        std::cout << "  ✓ Bookmark count correct: " << manager.count() << std::endl;
    } else {
        std::cout << "  ✗ Bookmark count incorrect" << std::endl;
        manager.remove(test_url1);
        manager.remove(test_url2);
        manager.remove(test_url3);
        return 1;
    }

    // Test 5: Remove bookmark
    std::cout << "\n[Test 5] Remove bookmark..." << std::endl;

    bool removed = manager.remove(test_url2);
    if (removed && !manager.contains(test_url2) && manager.count() == original_count + 2) {
        std::cout << "  ✓ Bookmark removed successfully" << std::endl;
    } else {
        std::cout << "  ✗ Bookmark removal failed" << std::endl;
        manager.remove(test_url1);
        manager.remove(test_url3);
        return 1;
    }

    // Clean up test bookmarks
    std::cout << "\n[Cleanup] Removing test bookmarks..." << std::endl;
    manager.remove(test_url1);
    manager.remove(test_url3);

    if (manager.count() == original_count) {
        std::cout << "  ✓ Cleanup successful, restored to " << original_count << " bookmarks" << std::endl;
    } else {
        std::cout << "  ⚠ Cleanup may have issues" << std::endl;
    }

    std::cout << "\n=== All bookmark tests passed! ===" << std::endl;
    return 0;
}
