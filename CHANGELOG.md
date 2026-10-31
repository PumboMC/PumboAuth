# Changelog

All notable changes to PumboAuth, one section per release. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.3-beta] - 2026-10-30

### Added

- Own file for Pumpkin 0.2.0 on GitHub: `PumboAuth-Pumpkin-0.2.0-0.1.3-beta.wasm`.
- On Pumpkin, `config.yml` and `lang/<code>.yml` files you never edited get new options and texts with an update. An edited file stays, and the log names what it lacks.

### Changed

- The 26.3 file (the one on the Pumpkin market) is built for Pumpkin 0.2.1 and no longer loads on 0.2.0.
- The database cache takes up to 16 MiB instead of 1 GiB.

### Fixed

- The first start after a crash opens the database faster.

## [0.1.2-beta] - 2026-10-09

No changes in PumboAuth; released with PumboPerms 0.1.2-beta.

## [0.1.1-beta] - 2026-10-09

### Fixed

- Help pages: the page number works again (`/pa help 2`, the ▶ arrow).

## [0.1.0-beta.1] - 2026-10-09

First public beta.

[0.1.3-beta]: https://github.com/PumboMC/PumboAuth/compare/v0.1.2-beta...v0.1.3-beta
[0.1.2-beta]: https://github.com/PumboMC/PumboAuth/compare/v0.1.1-beta...v0.1.2-beta
[0.1.1-beta]: https://github.com/PumboMC/PumboAuth/compare/v0.1.0-beta.1...v0.1.1-beta
[0.1.0-beta.1]: https://github.com/PumboMC/PumboAuth/releases/tag/v0.1.0-beta.1
