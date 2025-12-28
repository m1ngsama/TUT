#include "../src/http_client.h"
#include <iostream>

int main() {
    std::cout << "Testing basic image download...\n";

    try {
        HttpClient client;
        std::cout << "Client created\n";

        client.add_image_download("https://httpbin.org/image/png", nullptr);
        std::cout << "Image queued\n";
        std::cout << "Pending: " << client.get_pending_image_count() << "\n";

        std::cout << "Client will be destroyed\n";
    } catch (const std::exception& e) {
        std::cerr << "Error: " << e.what() << "\n";
        return 1;
    }

    std::cout << "Test completed successfully\n";
    return 0;
}
