@done
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
  Scenario: Fetch decodes HTML entities
    Given a tool workspace with a web_fetch tool backed by a mock server
    And the mock web server returns HTML:
      """
      <p>Tom &amp; Jerry &lt;3 &gt; 2</p>
      """
    When the agent executes tool "web_fetch" with mock URL
    Then the [ToolResult] should contain "Tom & Jerry"
    And the [ToolResult] should contain "< 3 > 2"
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
