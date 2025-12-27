#include "http_client.h"
#include <iostream>
#include <chrono>
#include <thread>

int main() {
    std::cout << "=== TUT 2.0 HTTP Async Test ===" << std::endl;

    HttpClient client;

    // Test 1: Synchronous fetch
    std::cout << "\n[Test 1] Synchronous fetch..." << std::endl;
    auto response = client.fetch("https://example.com");
    if (response.is_success()) {
        std::cout << "  ✓ Status: " << response.status_code << std::endl;
        std::cout << "  ✓ Content-Type: " << response.content_type << std::endl;
        std::cout << "  ✓ Body length: " << response.body.length() << " bytes" << std::endl;
    } else {
        std::cout << "  ✗ Failed: " << response.error_message << std::endl;
        return 1;
    }

    // Test 2: Asynchronous fetch
    std::cout << "\n[Test 2] Asynchronous fetch..." << std::endl;
    client.start_async_fetch("https://example.com");

    int polls = 0;
    auto start = std::chrono::steady_clock::now();

    while (true) {
        auto state = client.poll_async();
        polls++;

        if (state == AsyncState::COMPLETE) {
            auto end = std::chrono::steady_clock::now();
            auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(end - start).count();

            auto result = client.get_async_result();
            std::cout << "  ✓ Completed in " << ms << "ms after " << polls << " polls" << std::endl;
            std::cout << "  ✓ Status: " << result.status_code << std::endl;
            std::cout << "  ✓ Body length: " << result.body.length() << " bytes" << std::endl;
            break;
        } else if (state == AsyncState::FAILED) {
            auto result = client.get_async_result();
            std::cout << "  ✗ Failed: " << result.error_message << std::endl;
            return 1;
        } else if (state == AsyncState::LOADING) {
            // Non-blocking poll
            std::this_thread::sleep_for(std::chrono::milliseconds(10));
        } else {
            std::cout << "  ✗ Unexpected state" << std::endl;
            return 1;
        }

        if (polls > 1000) {
            std::cout << "  ✗ Timeout" << std::endl;
            return 1;
        }
    }

    // Test 3: Cancel async
    std::cout << "\n[Test 3] Cancel async..." << std::endl;
    client.start_async_fetch("https://httpbin.org/delay/10");

    // Poll a few times then cancel
    for (int i = 0; i < 5; i++) {
        client.poll_async();
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }

    client.cancel_async();
    std::cout << "  ✓ Request cancelled" << std::endl;

    // Verify state is CANCELLED or IDLE
    if (!client.is_async_active()) {
        std::cout << "  ✓ No active request after cancel" << std::endl;
    } else {
        std::cout << "  ✗ Request still active after cancel" << std::endl;
        return 1;
    }

    std::cout << "\n=== All tests passed! ===" << std::endl;
    return 0;
}
