# Security Policy

Thanks for taking the time to look. Agency Agents writes files into developer-tool configuration directories, so security reports are welcome.

## Supported Versions

| Version | Supported |
|---------|-----------|
| `0.1.x` | Yes       |

This is a pre-1.0 project. Support windows may change as release packaging stabilizes.

## Reporting A Vulnerability

Email **yuri.shvets@example.com** with:

- a clear description of the issue
- impact
- steps to reproduce or proof of concept
- version or commit tested
- name/handle for credit, if desired

Please do not open a public GitHub issue for security reports.

## Response Time

Best effort:

- acknowledgement within 7 days
- initial assessment within 14 days
- fix or mitigation plan within 30 days for high/critical findings

## Scope

In scope:

- remote code execution in the app or IPC commands
- arbitrary file read/write through Tauri commands
- path traversal in install, backup, catalog, or import/export paths
- unsafe overwrite or uninstall behavior
- bypass of modified-file backup protections
- XSS in the webview
- token leakage from GitHub OAuth storage or IPC
- SSRF or unexpected outbound requests
- updater signature or artifact verification bypass
- incorrect tool path handling that writes outside documented destinations

Out of scope:

- vulnerabilities in third-party AI coding tools
- malicious agent content in a user-selected catalog clone
- vulnerabilities in macOS, Windows, Linux, WebKit, or system components
- social-engineering attacks
- attacks requiring an already-compromised local account
- bugs in the upstream `agency-agents` repo that do not affect app install/write behavior

## Disclosure Policy

Coordinated disclosure, 90-day default. If a fix takes longer, the reporter and maintainer can agree on an extension before the embargo expires.

## Security Posture

- Typed Tauri IPC.
- No frontend arbitrary shell bridge.
- Deterministic renderers for supported tools.
- Local ledger records app-managed writes.
- Modified installed files are backed up before destructive operations.
- GitHub token is stored in the platform keychain and is not returned to the frontend.
- Network features are gated by settings and feature boundaries.

## Hall Of Fame

Reporters who have responsibly disclosed security issues:

<!-- First reporter goes here. Add as: Name (handle) - short description, fix in commit/PR link -->

*(empty - be the first)*
