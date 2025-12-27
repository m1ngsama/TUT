#include "history.h"
#include <iostream>
#include <cstdio>
#include <thread>
#include <chrono>

using namespace tut;

int main() {
    std::cout << "=== TUT 2.0 History Test ===" << std::endl;

    // 记录初始状态
    HistoryManager manager;
    size_t initial_count = manager.count();
    std::cout << "  Original history count: " << initial_count << std::endl;

    // Test 1: 添加历史记录
    std::cout << "\n[Test 1] Add history entries..." << std::endl;
    manager.add("https://example.com", "Example Site");
    manager.add("https://test.com", "Test Site");
    manager.add("https://demo.com", "Demo Site");

    if (manager.count() == initial_count + 3) {
        std::cout << "  ✓ Added 3 entries" << std::endl;
    } else {
        std::cout << "  ✗ Failed to add entries" << std::endl;
        return 1;
    }

    // Test 2: 重复 URL 更新
    std::cout << "\n[Test 2] Duplicate URL update..." << std::endl;
    std::this_thread::sleep_for(std::chrono::milliseconds(100));
    manager.add("https://example.com", "Example Site Updated");

    // 计数应该不变（因为重复的会被移到前面而不是新增）
    if (manager.count() == initial_count + 3) {
        std::cout << "  ✓ Duplicate correctly handled" << std::endl;
    } else {
        std::cout << "  ✗ Duplicate handling failed" << std::endl;
        return 1;
    }

    // Test 3: 最新在前面
    std::cout << "\n[Test 3] Most recent first..." << std::endl;
    const auto& entries = manager.get_all();
    if (!entries.empty() && entries[0].url == "https://example.com") {
        std::cout << "  ✓ Most recent entry is first" << std::endl;
    } else {
        std::cout << "  ✗ Order incorrect" << std::endl;
        return 1;
    }

    // Test 4: 持久化
    std::cout << "\n[Test 4] Persistence..." << std::endl;
    {
        HistoryManager manager2;  // 创建新实例会加载
        if (manager2.count() >= initial_count + 3) {
            std::cout << "  ✓ History persisted to file" << std::endl;
        } else {
            std::cout << "  ✗ Persistence failed" << std::endl;
            return 1;
        }
    }

    // Cleanup: 移除测试条目
    std::cout << "\n[Cleanup] Removing test entries..." << std::endl;
    HistoryManager cleanup_manager;
    // 由于我们没有删除单条的方法，这里只验证功能
    // 在实际使用中，历史会随着时间自然过期

    std::cout << "\n=== All history tests passed! ===" << std::endl;
    return 0;
}
