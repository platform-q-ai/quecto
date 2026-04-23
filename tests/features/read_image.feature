@done
Feature: Read tool image support
  As an AI agent
  I want to read image files as base64-encoded content
  So that I can view and reason about images in the workspace

  Background:
    Given a tool workspace

  Scenario: Reading a PNG file returns base64 image block
    Given a PNG image file "screenshot.png" exists in the workspace
    When the agent executes tool "read" with args:
      | path | screenshot.png |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/png" block
    And the [ToolResult] should contain "Read image file"
    And the [ToolResult] should contain "image/png"

  Scenario: Reading a JPEG file returns base64 image block
    Given a JPEG image file "photo.jpg" exists in the workspace
    When the agent executes tool "read" with args:
      | path | photo.jpg |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/jpeg" block

  Scenario: Reading a JPEG file with .jpeg extension works
    Given a JPEG image file "photo.jpeg" exists in the workspace
    When the agent executes tool "read" with args:
      | path | photo.jpeg |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/jpeg" block

  Scenario: Reading a GIF file returns base64 image block
    Given a GIF image file "anim.gif" exists in the workspace
    When the agent executes tool "read" with args:
      | path | anim.gif |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/gif" block

  Scenario: Reading a WebP file returns base64 image block
    Given a WebP image file "icon.webp" exists in the workspace
    When the agent executes tool "read" with args:
      | path | icon.webp |
    Then the [ToolResult] should not be an error
    And the [ToolResult] image blocks should contain a "image/webp" block

  Scenario: Reading a text file does not produce image blocks
    Given a file "notes.txt" exists with content "hello world"
    When the agent executes tool "read" with args:
      | path | notes.txt |
    Then the [ToolResult] should not be an error
    And the [ToolResult] should contain "hello world"
    And the [ToolResult] image blocks should be empty

  Scenario: Image file respects sandbox — path outside workspace is blocked
    Given a PNG image file "screenshot.png" exists in the workspace
    When the agent executes tool "read" with args:
      | path | /etc/hosts |
    Then the [ToolResult] should be an error
