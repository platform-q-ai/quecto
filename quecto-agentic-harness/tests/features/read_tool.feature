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
    Then the tool result should not be an error
    And the tool result image blocks should contain a "image/png" block

  @done
  Scenario: Magic bytes detect JPEG with wrong extension (.dat)
    Given a JPEG image file "photo.dat" exists in the workspace
    When the agent executes tool "read" with args:
      | path | photo.dat |
    Then the tool result should not be an error
    And the tool result image blocks should contain a "image/jpeg" block

  @done
  Scenario: Magic bytes detect WebP with no extension
    Given a WebP image file "icon" exists in the workspace
    When the agent executes tool "read" with args:
      | path | icon |
    Then the tool result should not be an error
    And the tool result image blocks should contain a "image/webp" block

  @done
  Scenario: Magic bytes detect GIF with wrong extension
    Given a GIF image file "anim.bin" exists in the workspace
    When the agent executes tool "read" with args:
      | path | anim.bin |
    Then the tool result should not be an error
    And the tool result image blocks should contain a "image/gif" block

  @done
  Scenario: Text file with .jpg extension is not mis-detected as image
    Given a file "not_an_image.jpg" exists with content "hello world"
    When the agent executes tool "read" with args:
      | path | not_an_image.jpg |
    Then the tool result should not be an error
    And the [ToolResult] image blocks should be empty
    And the tool result should contain "hello world"

  # --- Images sent as-is without client-side resize (#368) ---

  @done
  Scenario: Image is sent as-is without client-side resize
    Given a PNG image file "photo.png" exists in the workspace
    When the agent executes tool "read" with args:
      | path | photo.png |
    Then the tool result should not be an error
    And the tool result image blocks should contain a "image/png" block
    And the tool result should contain "Read image file"

  # --- Unchanged text read cache (#1522) ---

  @done
  Scenario: Re-reading unchanged text returns a short marker without repeating content
    Given a file "cache.txt" exists with content "alpha\nbeta\n"
    When the agent executes tool "read" with args:
      | path | cache.txt |
    And the agent executes tool "read" with args:
      | path | cache.txt |
    Then the tool result should not be an error
    And the tool result should contain "unchanged since read"
    And the tool result should not contain "alpha"

  @done
  Scenario: Force returns content for an unchanged text re-read
    Given a file "force-cache.txt" exists with content "alpha\nbeta\n"
    When the agent executes tool "read" with args:
      | path | force-cache.txt |
    And the agent executes tool "read" with args:
      | path  | force-cache.txt |
      | force | true            |
    Then the tool result should not be an error
    And the tool result should contain "alpha"
    And the tool result should not contain "unchanged since read"

  @done
  Scenario: A modified text re-read returns the new content before later short-circuiting
    Given a file "changed-cache.txt" exists with content "old\n"
    When the agent executes tool "read" with args:
      | path | changed-cache.txt |
    Given a file "changed-cache.txt" exists with content "new\n"
    When the agent executes tool "read" with args:
      | path | changed-cache.txt |
    Then the tool result should not be an error
    And the tool result should contain "new"
    And the tool result should not contain "unchanged since read"
    When the agent executes tool "read" with args:
      | path | changed-cache.txt |
    Then the tool result should not be an error
    And the tool result should contain "unchanged since read"

  @done
  Scenario: Different text ranges are cached independently
    Given a file "range-cache.txt" exists with 10 lines
    When the agent executes tool "read" with args:
      | path  | range-cache.txt |
      | limit | 2               |
    And the agent executes tool "read" with args:
      | path   | range-cache.txt |
      | offset | 3               |
      | limit  | 2               |
    Then the tool result should not be an error
    And the tool result should contain "line3"
    And the tool result should not contain "unchanged since read"
    When the agent executes tool "read" with args:
      | path   | range-cache.txt |
      | offset | 3               |
      | limit  | 2               |
    Then the tool result should not be an error
    And the tool result should contain "unchanged since read"
    When the agent executes tool "read" with args:
      | path   | range-cache.txt |
      | offset | 5               |
      | limit  | 2               |
    Then the tool result should not be an error
    And the tool result should contain "line5"
    And the tool result should not contain "unchanged since read"

  @done
  Scenario: Re-reading an image still returns an image block
    Given a PNG image file "cache-image.png" exists in the workspace
    When the agent executes tool "read" with args:
      | path | cache-image.png |
    And the agent executes tool "read" with args:
      | path | cache-image.png |
    Then the tool result should not be an error
    And the tool result image blocks should contain a "image/png" block
    And the tool result should contain "Read image file"

  # --- Truncation notice formatting ---

  @done
  Scenario: Byte-truncated file notice includes 50KB limit hint
    Given a file "bytes.txt" exists with 60000 bytes of content
    When the agent executes tool "read" with args:
      | path | bytes.txt |
    Then the tool result should not be an error
    And the tool result should contain "50KB limit"

  @done
  Scenario: User-specified limit notice shows remaining lines
    Given a file "big.txt" exists with 100 lines
    When the agent executes tool "read" with args:
      | path  | big.txt |
      | limit | 10      |
    Then the tool result should not be an error
    And the tool result should contain "more lines in file"
