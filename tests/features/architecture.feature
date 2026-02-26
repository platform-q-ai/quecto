@done @architecture
Feature: Architecture boundaries and ports
  As a maintainer
  I want architecture contracts encoded in executable tests
  So that boundary regressions are caught before refactors land

  Scenario: Domain channel port is dyn-compatible
    Then the source file "src/domain/channel.rs" should contain "Pin<Box<dyn Future"
    And the source file "src/domain/channel.rs" should not contain "-> impl std::future::Future"

  Scenario: Application layer avoids direct runtime I/O
    Then the application source should not contain runtime I/O patterns

  Scenario: Gateway runtime uses trait ports instead of concrete implementations
    Then the source file "src/interface/gateway/mod.rs" should not contain "pub(super) agent: Arc<AgentLoopImpl>"
    And the source file "src/interface/gateway/mod.rs" should not contain "pub(super) session_store: Arc<FileSessionStore>"
    And the source file "src/interface/gateway/mod.rs" should not contain "pub(super) telegram: TelegramChannel"
    And the source file "src/interface/gateway/services.rs" should not contain "agent: Arc<AgentLoopImpl>"
    And the source file "src/interface/gateway/services.rs" should not contain "session_store: Arc<FileSessionStore>"
    And the source file "src/interface/gateway/services.rs" should not contain "telegram: TelegramChannel"
