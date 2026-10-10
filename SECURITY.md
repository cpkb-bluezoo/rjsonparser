# Security Policy

## Supported Versions

rjsonparser does not yet maintain parallel maintenance branches. Security
fixes are made against the latest release and the `main` branch; older
releases are not backported.

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

Please **do not** open a public GitHub issue for security vulnerabilities.

The preferred way to report a vulnerability is through GitHub's private
vulnerability reporting:

1. Go to the [Security tab](https://github.com/cpkb-bluezoo/rjsonparser/security) of this repository.
2. Click **Report a vulnerability**.
3. Provide as much detail as possible: affected version(s), a minimal
   reproduction (ideally a small JSON document and the API call that
   triggers the issue), and the observed impact.

If you are unable to use GitHub's private reporting, you may instead email
the maintainer directly at **dog@gnu.org**. Please include "rjsonparser
security" in the subject line, and PGP-encrypt the report if it contains
proof-of-concept exploit code.

You should expect an initial response within a few days acknowledging
receipt. rjsonparser is currently maintained by a single volunteer
maintainer, so please be patient - a fix timeline will be communicated once
the report has been triaged, and credit will be given in the release notes
unless you ask to remain anonymous. Coordinated disclosure is appreciated:
please give the maintainer a reasonable window to publish a fix before
disclosing publicly.

## Scope

rjsonparser is a JSON parsing and serialization library for Rust, so reports of
particular interest include (but are not limited to):

- Parser crashes, hangs, or excessive memory/CPU consumption on malformed
  or adversarial input (denial of service)
- Stack overflow via deeply nested arrays/objects
- Integer/number parsing issues (overflow, precision loss leading to
  unsafe application-level decisions)
- Incorrect handling of duplicate object keys, or of control/escape
  characters, that could lead a consuming application to misinterpret
  input it believes has already been validated
- Any memory-safety or arbitrary-code-execution issue (the crate is safe Rust; a panic on untrusted input counts as a denial of service)
