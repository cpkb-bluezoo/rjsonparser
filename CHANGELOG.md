# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-10-10

### Changed

- Relicensed from LGPL-2.1-or-later to LGPL-3.0-or-later, matching jsonparser
  and rmimeparser. `LICENSE` is now the LGPL v3 text and `LICENSE-GPL` carries
  the GPL v3 it incorporates.
- **Breaking:** `Parser` now owns its handler (`Parser<H>`, previously `Parser<'a, H>` over
  `&'a mut H`). Passing `&mut handler` or a `Box<dyn ContentHandler>` still
  works; `handler()`, `handler_mut()` and `into_handler()` give access back.

### Fixed

- Floating-point literals are now rounded exactly. The previous
  `mantissa * 10^exp` fast path, inherited from the Java tokenizer, could be
  off by one ulp (e.g. `1.5e-5`). The fast path is now exact (integer
  mantissa below 2^53, power of ten up to 10^22) and everything else goes
  through `str::parse::<f64>`.

- A large string or key arriving in small chunks no longer costs quadratic
  time. `receive()` re-copied the whole pending leftover and re-scanned the
  incomplete token from its start on every call, so a 2 MB string in 1 KB
  chunks took 10-40 s (a 20 MB string, within the default limit, far
  longer). The leftover is now drained rather than rebuilt, and string
  scanning resumes where it stopped, including part-way through an escape
  sequence. The same input now parses in milliseconds. Numbers still
  re-scan, but are bounded by `max_number_length` (default 1000).

### Baseline

The port tracks jsonparser 1.3 (14 Jul 2026): configurable resource limits,
opt-in duplicate-key rejection, key interning with adaptive bail-out, and
JSONTestSuite conformance. jsonparser has had no source changes since 1.3.
