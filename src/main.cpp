#include "tui_view.h"
#include "calendar.h"

int main() {
    while (true) {
        int choice = run_portal_tui();

        switch (choice) {
            case 0: { // Calendar
                Calendar calendar;
                calendar.run();
                break;
            }
            case 1: { // Exit
                return 0;
            }
        }
    }

    return 0;
}

