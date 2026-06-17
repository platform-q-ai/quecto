Feature: ReadTool — Quecto compatibility
  As an AI agent
  I want the read tool to match Quecto's feature set
  So that LLM interactions are reliable regardless of image format or file size

  Background:
    Given a tool workspace

  # --- Magic-byte image MIME detection ---

  @done
  Scenario: Magic bytes detect PNG with no extension
    Given a PNG image file "screenshot" exists in the workspace
    When the agent executes tool "read" with args:
      | path | screenshot |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/png" block

  @done
  Scenario: Magic bytes detect JPEG with wrong extension (.dat)
    Given a JPEG image file "photo.dat" exists in the workspace
    When the agent executes tool "read" with args:
      | path | photo.dat |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/jpeg" block

  @done
  Scenario: Magic bytes detect WebP with no extension
    Given a WebP image file "icon" exists in the workspace
    When the agent executes tool "read" with args:
      | path | icon |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/webp" block

  @done
  Scenario: Magic bytes detect GIF with wrong extension
    Given a GIF image file "anim.bin" exists in the workspace
    When the agent executes tool "read" with args:
      | path | anim.bin |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/gif" block

  @done
  Scenario: Text file with .jpg extension is not mis-detected as image
    Given a file "not_an_image.jpg" exists with content "hello world"
    When the agent executes tool "read" with args:
      | path | not_an_image.jpg |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should be empty
    And the [ToolResult] should contain "hello world"

  # --- Images sent as-is without client-side resize (#368) ---

  @done
  Scenario: Image is sent as-is without client-side resize
    Given a PNG image file "photo.png" exists in the workspace
    When the agent executes tool "read" with args:
      | path | photo.png |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/png" block
    And the [ToolResult] should contain "Read image file"

  # --- Truncation notice formatting ---

  @done
  Scenario: Byte-truncated file notice includes 50KB limit hint
    Given a file "bytes.txt" exists with 60000 bytes of content
    When the agent executes tool "read" with args:
      | path | bytes.txt |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "50KB limit"

  @done
  Scenario: User-specified limit notice shows remaining lines
    Given a file "big.txt" exists with 100 lines
    When the agent executes tool "read" with args:
      | path  | big.txt |
      | limit | 10      |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "more lines in file"
