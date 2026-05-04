# Boards runtime config workflow smoke test

This smoke document verifies the end-to-end Boards MCP tool path for the deployed Quecto runtime-manager.

The workflow confirms that:

- the deployed runtime-manager can provision a Quecto pod with a custom workflow configuration;
- Quecto loads the custom Boards workflow with `--config`;
- prompt submission is asynchronous with `waitForCompletion=false`;
- the repository workflow can create a docs-only change on branch `boards-runtime-config-e2e-1777896990` for PR validation.
