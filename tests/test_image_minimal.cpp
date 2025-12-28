#include "../src/http_client.h"
#include <iostream>
#include <thread>
#include <chrono>

int main() {
    std::cout << "=== Minimal Async Image Test ===\n";

    try {
        HttpClient client;
        std::cout << "1. Client created\n";

        client.add_image_download("https://httpbin.org/image/png", nullptr);
        std::cout << "2. Image queued\n";
        std::cout << "   Pending: " << client.get_pending_image_count() << "\n";

        std::cout << "3. First poll...\n";
        client.poll_image_downloads();
        std::cout << "   After poll - Pending: " << client.get_pending_image_count()
                 << ", Loading: " << client.get_loading_image_count() << "\n";

        std::cout << "4. Wait a bit...\n";
        std::this_thread::sleep_for(std::chrono::milliseconds(500));

        std::cout << "5. Second poll...\n";
        client.poll_image_downloads();
        std::cout << "   After poll - Pending: " << client.get_pending_image_count()
                 << ", Loading: " << client.get_loading_image_count() << "\n";

        std::cout << "6. Get completed...\n";
        auto completed = client.get_completed_images();
        std::cout << "   Completed: " << completed.size() << "\n";

        std::cout << "7. Destroying client...\n";
    } catch (const std::exception& e) {
        std::cerr << "ERROR: " << e.what() << "\n";
        return 1;
    }

    std::cout << "8. Test completed successfully!\n";
    return 0;
}
