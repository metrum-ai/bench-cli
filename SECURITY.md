<!-- Copyright (c) 2026 Metrum AI, Inc. Licensed under the Apache License, Version 2.0. -->

# Security Policy

## Supported versions

Only the latest release on the default branch receives security fixes.

## Reporting a vulnerability

Please do not report security vulnerabilities through public GitHub issues.

Use GitHub's private vulnerability reporting for this repository
(Security tab, "Report a vulnerability"), or email opensource@metrum.ai.
Include the affected version, a description of the issue, and reproduction
steps. You will receive an acknowledgement within five business days.

## Scope

Metrum AI Bench is a load-generation client. It sends the prompts, images and audio
you supply to the endpoints you specify, with the API keys you provide. Keys
are passed on the command line or in an endpoints file; treat those files and
your shell history accordingly. The debug log at `--log-level debug` may
contain request payloads; do not share it without review.

First-party Rust sources under `src/` contain no `unsafe` blocks.

Findings about the correctness of published metrics are not security issues;
open a regular issue for those.
