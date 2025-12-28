#include "../src/http_client.h"
#include <iostream>
#include <cassert>
#include <chrono>
#include <thread>

void test_async_image_downloads() {
    std::cout << "Testing async image downloads...\n";

    HttpClient client;

    // Add multiple image downloads
    // Using small test images from httpbin.org
    const char* test_images[] = {
        "https://httpbin.org/image/png",
        "https://httpbin.org/image/jpeg",
        "https://httpbin.org/image/webp"
    };

    // Add images to queue
    for (int i = 0; i < 3; i++) {
        client.add_image_download(test_images[i], (void*)(intptr_t)i);
        std::cout << "  Queued: " << test_images[i] << "\n";
    }

    std::cout << "  Pending: " << client.get_pending_image_count() << "\n";
    assert(client.get_pending_image_count() == 3);

    // Poll until all images are downloaded
    int iterations = 0;
    int max_iterations = 200;  // 10 seconds max (50ms * 200)

    while ((client.get_pending_image_count() > 0 ||
            client.get_loading_image_count() > 0) &&
           iterations < max_iterations) {
        client.poll_image_downloads();

        // Check for completed images
        auto completed = client.get_completed_images();
        for (const auto& img : completed) {
            std::cout << "  Downloaded: " << img.url
                     << " (status: " << img.status_code
                     << ", size: " << img.data.size() << " bytes)\n";
        }

        std::this_thread::sleep_for(std::chrono::milliseconds(50));
        iterations++;
    }

    std::cout << "  Completed after " << iterations << " iterations\n";
    std::cout << "  Final state - Pending: " << client.get_pending_image_count()
             << ", Loading: " << client.get_loading_image_count() << "\n";

    std::cout << "✓ Async image download test passed!\n\n";
}

void test_image_cancellation() {
    std::cout << "Testing image download cancellation...\n";

    HttpClient client;

    // Add images
    client.add_image_download("https://httpbin.org/image/png", nullptr);
    client.add_image_download("https://httpbin.org/image/jpeg", nullptr);

    std::cout << "  Queued 2 images\n";

    // Start downloads
    client.poll_image_downloads();
    std::this_thread::sleep_for(std::chrono::milliseconds(100));

    // Cancel all
    std::cout << "  Cancelling all downloads\n";
    client.cancel_all_images();

    assert(client.get_pending_image_count() == 0);
    assert(client.get_loading_image_count() == 0);

    std::cout << "✓ Image cancellation test passed!\n\n";
}

void test_concurrent_limit() {
    std::cout << "Testing concurrent download limit...\n";

    HttpClient client;
    client.set_max_concurrent_images(2);

    // Add 5 images
    for (int i = 0; i < 5; i++) {
        client.add_image_download("https://httpbin.org/delay/1", nullptr);
    }

    std::cout << "  Queued 5 images with max_concurrent=2\n";
    assert(client.get_pending_image_count() == 5);

    // Start downloads
    client.poll_image_downloads();

    // Should have max 2 loading at a time
    int loading = client.get_loading_image_count();
    std::cout << "  Loading: " << loading << " (should be <= 2)\n";
    assert(loading <= 2);

    client.cancel_all_images();
    std::cout << "✓ Concurrent limit test passed!\n\n";
}

int main() {
    std::cout << "=== Async Image Download Tests ===\n\n";

    try {
        test_async_image_downloads();
        test_image_cancellation();
        test_concurrent_limit();

        std::cout << "=== All tests passed! ===\n";
        return 0;
    } catch (const std::exception& e) {
        std::cerr << "Test failed: " << e.what() << "\n";
        return 1;
    }
}
