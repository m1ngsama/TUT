#!/bin/bash

# TUT Browser Interactive Test Script
# This script helps test the browser with various real websites

echo "========================================"
echo "  TUT 2.0 Browser Interactive Testing"
echo "========================================"
echo ""
echo "This script will help you test the browser with real websites."
echo "Press Ctrl+C to exit at any time."
echo ""

# Build the browser
echo "Building the browser..."
cmake -B build -S . -DCMAKE_BUILD_TYPE=Debug > /dev/null 2>&1
cmake --build build > /dev/null 2>&1

if [ $? -ne 0 ]; then
    echo "❌ Build failed!"
    exit 1
fi

echo "✓ Build successful!"
echo ""

# Test sites
declare -a sites=(
    "http://example.com"
    "http://info.cern.ch"
    "http://motherfuckingwebsite.com"
    "http://textfiles.com"
)

echo "Available test sites:"
echo "1. example.com - Simple static page"
echo "2. info.cern.ch - First website ever (historical)"
echo "3. motherfuckingwebsite.com - Minimalist design manifesto"
echo "4. textfiles.com - Text-only content"
echo "5. Custom URL"
echo ""

read -p "Select a site (1-5): " choice

case $choice in
    1) url="${sites[0]}" ;;
    2) url="${sites[1]}" ;;
    3) url="${sites[2]}" ;;
    4) url="${sites[3]}" ;;
    5)
        read -p "Enter URL (include http://): " url
        ;;
    *)
        echo "Invalid choice, using example.com"
        url="${sites[0]}"
        ;;
esac

echo ""
echo "========================================"
echo "  Launching TUT Browser"
echo "========================================"
echo "URL: $url"
echo ""
echo "Keyboard shortcuts:"
echo "  j/k - Scroll up/down"
echo "  Tab - Next link/field"
echo "  Enter - Follow link/activate field"
echo "  i - Focus first form field"
echo "  / - Search"
echo "  h/l - Back/Forward"
echo "  B - Bookmark"
echo "  :o URL - Open URL"
echo "  :q - Quit"
echo ""
read -p "Press Enter to start..."

# Launch the browser
./build/tut "$url"

echo ""
echo "Browser exited. Test complete!"
