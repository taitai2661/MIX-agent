# Security Policy

MIX-agent is a local-first workspace: the Desktop edition runs with the
signed-in OS user's permissions, and the Docker edition is intended for a
trusted LAN or VPN. Neither edition is designed to execute hostile code or to
be exposed directly to the public internet.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem.

Use GitHub's private vulnerability reporting:
<https://github.com/taitai2661/MIX-agent/security/advisories/new>

Include the affected edition and version, a description of the impact, and the
smallest reproduction you can share. You can expect an acknowledgement within
a few days. Please give us a reasonable window to ship a fix before publishing
details.

## Supported versions

The latest release is supported. Older releases may receive a fix when the
issue is severe and the patch applies cleanly.

## Scope notes

The following are known, documented design boundaries rather than
vulnerabilities:

* The Desktop edition trusts the OS user. Workspace tools run with that user's
  permissions; approval rules reduce accidental damage, not a hostile local
  account.
* File and command access that was not chosen through a native picker requires
  an explicit in-app confirmation. Report any path that is read or written
  **without** that prompt.
* The Docker edition ships with authentication enabled after the first-run
  setup. Exposing port 8080 to the internet without a TLS-terminating proxy,
  `PUBLIC_ORIGIN`, and network restrictions is unsupported.
* Model output is untrusted. Tool calls still go through the approval
  pipeline; a bypass of `deny` rules or of the egress allowlist is in scope.

Please include the edition (Desktop or Docker), OS, and version in every
report.
