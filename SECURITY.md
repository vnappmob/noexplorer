# Security policy

NoExplorer handles S3 credentials and moves user files. Please report vulnerabilities
privately: **kris.luu@vnappmob.com**, or GitHub → Security → *Report a vulnerability*.
Don't open public issues for security problems.

We aim to acknowledge reports within 3 business days.

Scope notes:
- Credentials are stored in a local JSON file with 0600 permissions (see ADR-008).
- Object keys are treated as untrusted and sanitized before becoming local paths (`util::safe_rel`).
- See "Security model" in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#security-model).
