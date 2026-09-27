@done @web-fetch
Feature: Web Fetch Tool
  As an AI agent
  I want to fetch web pages and extract readable text
  So that I can research topics without overwhelming my context with HTML

  # ─── Basic fetch ────────────────────────────────────────────────────────────

  @done
  Scenario: Fetch HTML page strips tags and returns text
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTML:
      """
      <html><head><title>Test</title></head>
      <body><h1>Hello</h1><p>World</p></body></html>
      """
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should contain "Hello"
    And the [ToolResult] should contain "World"
    And the [ToolResult] should not contain "<h1>"
    And the [ToolResult] should not be an error

  @done
  Scenario: Fetch strips script, style, nav, footer, and header blocks
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTML:
      """
      <html><body>
      <script>alert('xss')</script>
      <style>.foo { color: red; }</style>
      <nav>Menu Items</nav>
      <header>Header Content</header>
      <main><p>Main Content</p></main>
      <footer>Footer Content</footer>
      </body></html>
      """
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should contain "Main Content"
    And the [ToolResult] should not contain "alert"
    And the [ToolResult] should not contain "color"
    And the [ToolResult] should not contain "Menu Items"
    And the [ToolResult] should not contain "Header Content"
    And the [ToolResult] should not contain "Footer Content"
    And the [ToolResult] should not be an error

  @done
  Scenario: Fetch keeps only the main content of a page that marks one
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTML:
      """
      <html><head><title>Install Guide</title></head><body>
      <aside><p>Popular posts</p><p>Archive list</p><p>Newsletter signup</p></aside>
      <main><h1>Install</h1><p>Download the archive and run the installer, then restart your shell so the new path is picked up.</p></main>
      </body></html>
      """
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should contain "Install Guide"
    And the [ToolResult] should contain "Main content only"
    And the [ToolResult] should contain "bytes of page text dropped; main_only: false returns the whole page"
    And the [ToolResult] should contain "run the installer"
    And the [ToolResult] should not contain "Popular posts"
    And the [ToolResult] should not be an error

  # #2225: documentation pages keep over nine tenths of their text in
  # <main>; their navigation is still dropped.
  @done
  Scenario: Fetch keeps the main content even when it holds nearly all the text
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTML:
      """
      <html><head><title>Array.map</title></head><body>
      <div class="sidebar"><span>Navigation</span><a href="/a">Arrays</a><a href="/b">Maps</a></div>
      <main><h1>Array.prototype.map()</h1><p>The map() method of Array instances creates a new array populated with the results of calling a provided function on every element in the calling array. It does not change the array it is called on, and it skips the empty slots of sparse arrays.</p></main>
      </body></html>
      """
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should contain "Main content only; 20 bytes of page text dropped"
    And the [ToolResult] should contain "creates a new array"
    And the [ToolResult] should not contain "Navigation"
    And the [ToolResult] should not be an error

  # #2225: the whole page keeps adjacent labels apart.
  @done
  Scenario: The whole page keeps adjacent labels apart
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTML:
      """
      <html><body><div><span>Navigation</span><a href="/a">Introduction to Node.js</a><a href="/b">Getting Started</a></div></body></html>
      """
    When the agent executes tool "web_fetch" with mock URL and main_only false
    Then the [ToolResult] should contain "Navigation Introduction to Node.js Getting Started"
    And the [ToolResult] should not be an error

  @done
  Scenario: Fetch decodes HTML entities
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTML:
      """
      <p>Tom &amp; Jerry &lt;3 &gt; 2</p>
      """
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should contain "Tom & Jerry"
    And the [ToolResult] should contain "<3 > 2"
    And the [ToolResult] should not be an error

  # ─── Raw mode ───────────────────────────────────────────────────────────────

  @done
  Scenario: Raw mode returns body without stripping
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns body:
      """
      {"key":"value","items":[1,2,3]}
      """
    When the agent executes tool "web_fetch" with mock URL and raw mode
    Then the [ToolResult] should contain "\"key\":\"value\""
    And the [ToolResult] should not be an error

  @done
  Scenario: Raw mode preserves HTML tags
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTML:
      """
      <h1>Keep Tags</h1>
      """
    When the agent executes tool "web_fetch" with mock URL and raw mode
    Then the [ToolResult] should contain "<h1>Keep Tags</h1>"
    And the [ToolResult] should not be an error

  # ─── Truncation ─────────────────────────────────────────────────────────────

  @done
  Scenario: Large response is truncated to max_response_kb
    Given a tool workspace with a web_fetch tool backed by a mock server with 1KB limit
    And the mock web server returns a 4KB plain text body
    When the agent executes tool "web_fetch" with mock URL and raw mode
    Then the [ToolResult] should contain "[Truncated"
    And the [ToolResult] should not be an error

  # ─── Error handling ─────────────────────────────────────────────────────────

  @done
  Scenario: HTTP 404 returns error result
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTTP 404
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "404"

  @done
  Scenario: HTTP 500 returns error result
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTTP 500
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "500"

  @done
  Scenario: Missing URL parameter returns error
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with empty args
    Then the [ToolResult] should be a domain error

  @done
  Scenario: Invalid JSON returns error
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with raw args "not json"
    Then the [ToolResult] should be a domain error

  # ─── Scheme validation ──────────────────────────────────────────────────────

  @done
  Scenario: FTP scheme is rejected
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | ftp://example.com/file |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "Invalid URL scheme"

  @done
  Scenario: File scheme is rejected
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | file:///etc/passwd |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "Invalid URL scheme"

  # ─── SSRF protection ───────────────────────────────────────────────────────

  @done
  Scenario: Localhost URL is blocked
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | http://localhost/secret |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "restricted"

  @done
  Scenario: Loopback IP is blocked
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | http://127.0.0.1/secret |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "restricted"

  @done
  Scenario: Private RFC-1918 IP is blocked
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | http://10.0.0.1/internal |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "restricted"

  @done
  Scenario: AWS metadata IP is blocked
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | http://169.254.169.254/latest/meta-data/ |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "restricted"

  @done
  Scenario: IPv6 loopback is blocked
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | http://[::1]/secret |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "restricted"

  @done
  Scenario: Google Cloud metadata domain is blocked
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | http://metadata.google.internal/computeMetadata/v1/ |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "restricted"

  @done
  Scenario: IPv4-mapped cloud metadata address is blocked
    Given a tool workspace with a web_fetch tool backed by a mock server
    When the agent executes tool "web_fetch" with args:
      | url | http://[::ffff:a9fe:a9fe]/latest/meta-data/ |
    Then the [ToolResult] should be an error
    And the [ToolResult] should contain "refused: 169.254.169.254 (via ::ffff:169.254.169.254) is not a public address"

  @done
  Scenario: Redirect to a restricted address is refused before it is followed
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server redirects to itself through an IPv4-mapped address
    When the agent executes tool "web_fetch" with mock URL
    Then the tool call should fail with "restricted address; refused: the redirect to http://[::ffff:7f00:1]:"
    And the mock web server should have received 1 request

  # #2209: a failed hop is named, with the cause, not the URL asked for.
  @done
  Scenario: A redirect to a closed port names the hop and the cause
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server redirects to a closed port
    When the agent executes tool "web_fetch" with mock URL
    Then the tool call should fail with "Fetch failed: connection refused on the redirect hop to http://web-fetch.test:"
    And the tool call should fail with "Connection refused"

  # ─── Plain text passthrough ─────────────────────────────────────────────────

  @done
  Scenario: Plain text content passes through without mangling
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns body:
      """
      Just plain text, no HTML tags at all.
      """
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should contain "Just plain text, no HTML tags at all."
    And the [ToolResult] should not be an error

  # ─── Tool definition ───────────────────────────────────────────────────────

  @done
  Scenario: Tool definition has correct name and schema
    Given a tool workspace with a web_fetch tool backed by a mock server
    Then the tool registry should contain "web_fetch"
