# Security Policy

Compass is a launcher that runs extensions in a sandboxed environment and handles user credentials, clipboard data, and application permissions. We take security seriously and welcome vulnerability reports.

## Reporting a vulnerability

**Do not open a public issue for security vulnerabilities.** Instead, use GitHub's private vulnerability reporting:

1. Go to the [Compass security advisories page](https://github.com/tuna-os/compass/security/advisories/new).
2. Click **Report a vulnerability** and draft a security advisory.
3. Include:
   - Affected component (engine, extension host, sandbox, platform integration, etc.)
   - Minimal steps to reproduce
   - Impact and severity (privilege escalation, data leakage, sandbox escape, etc.)
   - Suggested fix, if you have one

Alternatively, email the maintainers through a GitHub Security Advisory draft if the affected version is not yet released.

## What to report

Compass has specific threat boundaries. Report issues in:

- **Sandbox escapes**: extensions breaking out of the Landlock filesystem boundary or seccomp filter
- **Extension privilege escalation**: extensions gaining unintended access to APIs, permissions, or host files
- **Credential leaks**: insecure handling of OAuth tokens, passwords, SSH keys, or other secrets
- **Clipboard abuse**: extensions or scripts reading clipboard contents without explicit user consent
- **IPC vulnerabilities**: command injection or privilege escalation through the Compass-extension bridge
- **Wayland/X11 integration**: screen capture, input injection, or window snooping
- **Build and supply chain**: unsigned artifacts, unpinned dependencies, or tampered releases

## Out of scope

Do not report vulnerabilities in:

- **Upstream projects**: Raycast SDK, Vicinae, Iced, Rust standard library, GNOME/KDE, systemd, D-Bus, Wayland compositors. Report those to their maintainers.
- **Third-party extensions**: extensions from the Raycast or extension store. Report issues to the extension author.
- **Host environment**: container runtimes, the OS kernel, glibc, OpenSSL. Report to the relevant project.

## Response timeline

We aim to:

- **Acknowledge** your report within **5 business days**
- **Triage** the vulnerability and confirm reproducibility within **10 days**
- **Fix and release** a patch through the normal release pipeline, usually within **30 days** of triage
- **Coordinate disclosure**: we prefer coordinated disclosure. Please give us a reasonable window (at least 30 days after we confirm a fix is ready) before publishing details publicly

## Disclosure

Once a fix is released, we will:

1. Publish a GitHub Security Advisory with CVE details
2. Add an entry to the CHANGELOG with the version and fix summary
3. Notify users through release notes

If you discovered the vulnerability responsibly and wish to be credited, tell us in your report.

## Security model notes

**The Compass sandbox is defense-in-depth, not a jail.** Extensions run behind:
- Landlock filesystem boundary (readable paths: `~/.ssh/config`, `~/.config/compass/`, `/usr/share/`, and others listed in [`crates/compass-sandbox`](crates/compass-sandbox))
- seccomp filter (blocks certain syscalls; see [ADR-0013](docs/rust-engine/adr/0013-extension-sandbox.md))
- Memory cap (configurable per extension)

A determined attacker with shell access or kernel vulnerabilities can likely escape. The sandbox protects against buggy or careless extensions, not malicious ones run with host privileges.

**Extension permissions are explicit.** Extensions running on Compass must declare capabilities (clipboard access, subprocess execution, file I/O) and you approve them. Raycast extensions ported to Compass may have fewer host APIs available (see [RAYCAST-LINUX-SHIM.md](docs/rust-engine/RAYCAST-LINUX-SHIM.md) for details).

**Compass checks for updates automatically but never auto-applies them.** Update checks are configurable (Settings → Updates); you control when and whether to install updates.

## Questions or feedback

If you have questions about this policy or feedback on Compass's security posture, open a public issue in the [Compass tracker](https://github.com/tuna-os/compass/issues) or reach out to the maintainers.
