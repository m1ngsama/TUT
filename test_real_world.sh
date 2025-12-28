#!/bin/bash
# Real-world browser testing script
# Tests TUT with various website types and provides UX feedback

echo "════════════════════════════════════════════════════════════"
echo "  TUT Real-World Browser Testing"
echo "════════════════════════════════════════════════════════════"
echo ""
echo "This script will test TUT with various website types:"
echo ""
echo "1. News sites (text-heavy, many images)"
echo "2. Documentation (code blocks, technical content)"
echo "3. Simple static sites (basic HTML)"
echo "4. Image galleries (many concurrent images)"
echo "5. Forums/discussions (mixed content)"
echo ""
echo "For each site, we'll evaluate:"
echo "  • Loading speed and responsiveness"
echo "  • Image loading behavior"
echo "  • Content readability"
echo "  • Navigation smoothness"
echo "  • Overall user experience"
echo ""
echo "════════════════════════════════════════════════════════════"
echo ""

# Test categories
declare -A SITES

# Category 1: News/Content sites
SITES["news_hn"]="https://news.ycombinator.com"
SITES["news_lobsters"]="https://lobste.rs"

# Category 2: Documentation
SITES["doc_curl"]="https://curl.se/docs/manpage.html"
SITES["doc_rust"]="https://doc.rust-lang.org/book/ch01-01-installation.html"

# Category 3: Simple/Static
SITES["simple_example"]="https://example.com"
SITES["simple_motherfuckingwebsite"]="https://motherfuckingwebsite.com"

# Category 4: Wikipedia (images + content)
SITES["wiki_unix"]="https://en.wikipedia.org/wiki/Unix"
SITES["wiki_web"]="https://en.wikipedia.org/wiki/World_Wide_Web"

# Category 5: Tech blogs
SITES["blog_lwn"]="https://lwn.net"

echo "Available test sites:"
echo ""
echo "News & Content:"
echo "  1. Hacker News         - ${SITES[news_hn]}"
echo "  2. Lobsters            - ${SITES[news_lobsters]}"
echo ""
echo "Documentation:"
echo "  3. curl manual         - ${SITES[doc_curl]}"
echo "  4. Rust Book           - ${SITES[doc_rust]}"
echo ""
echo "Simple Sites:"
echo "  5. Example.com         - ${SITES[simple_example]}"
echo "  6. Motherfucking Web   - ${SITES[simple_motherfuckingwebsite]}"
echo ""
echo "Wikipedia:"
echo "  7. Unix                - ${SITES[wiki_unix]}"
echo "  8. World Wide Web      - ${SITES[wiki_web]}"
echo ""
echo "Tech News:"
echo "  9. LWN.net             - ${SITES[blog_lwn]}"
echo ""
echo "════════════════════════════════════════════════════════════"
echo ""

# Function to test a site
test_site() {
    local name=$1
    local url=$2

    echo ""
    echo "──────────────────────────────────────────────────────────"
    echo "Testing: $name"
    echo "URL: $url"
    echo "──────────────────────────────────────────────────────────"
    echo ""
    echo "Starting browser... (Press 'q' to quit and move to next)"
    echo ""

    ./build/tut "$url"

    echo ""
    echo "Test completed for: $name"
    echo ""
}

# Interactive mode
echo "Select test mode:"
echo "  a) Test all sites automatically"
echo "  m) Manual site selection"
echo "  q) Quit"
echo ""
read -p "Choice: " choice

case $choice in
    a)
        echo ""
        echo "Running automated tests..."
        echo "Note: Each site will open. Press 'q' to move to next."
        echo ""
        sleep 2

        test_site "Hacker News" "${SITES[news_hn]}"
        test_site "Example.com" "${SITES[simple_example]}"
        test_site "Wikipedia - Unix" "${SITES[wiki_unix]}"
        ;;
    m)
        echo ""
        read -p "Enter site number (1-9): " num
        case $num in
            1) test_site "Hacker News" "${SITES[news_hn]}" ;;
            2) test_site "Lobsters" "${SITES[news_lobsters]}" ;;
            3) test_site "curl manual" "${SITES[doc_curl]}" ;;
            4) test_site "Rust Book" "${SITES[doc_rust]}" ;;
            5) test_site "Example.com" "${SITES[simple_example]}" ;;
            6) test_site "Motherfucking Website" "${SITES[simple_motherfuckingwebsite]}" ;;
            7) test_site "Wikipedia - Unix" "${SITES[wiki_unix]}" ;;
            8) test_site "Wikipedia - WWW" "${SITES[wiki_web]}" ;;
            9) test_site "LWN.net" "${SITES[blog_lwn]}" ;;
            *) echo "Invalid selection" ;;
        esac
        ;;
    q)
        echo "Exiting..."
        exit 0
        ;;
    *)
        echo "Invalid choice"
        exit 1
        ;;
esac

echo ""
echo "════════════════════════════════════════════════════════════"
echo "  Testing Complete!"
echo "════════════════════════════════════════════════════════════"
