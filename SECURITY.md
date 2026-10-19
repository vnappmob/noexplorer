# Security policy

NoExplorer handles S3 credentials and moves user files. Please report vulnerabilities
privately: **kris.luu@vnappmob.com**, or GitHub → Security → *Report a vulnerability*.
Don't open public issues for security problems.

We aim to acknowledge reports within 3 business days.

Scope notes:
- Secret keys, app passwords and Box tokens live in the OS credential store (Keychain, Credential
  Manager, Secret Service; ADR-020). `config.json` (mode 0600) holds them only when no store
  exists, and they never reach the UI or the logs.
- TLS is never switched off. Self-signed servers work only by pinning one certificate's SHA-256
  fingerprint per account, after the user compares it (experimental, ADR-032).
- Object keys are treated as untrusted and sanitized before becoming local paths (`util::safe_rel`).
- Untrusted strings are escaped before they reach `innerHTML`; text previews use `textContent`.
- See "Security model" in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#security-model).
