# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- External check execution through an application port and a Unix process adapter,
  with project registration, bounded JSON communication, timeout and cancellation
  cleanup, advisory results and mandatory change gates.
- Explicit `check_run` operations, saved external result freshness in `status`,
  and atomic SQLite storage of results and execution evidence alongside changes.
