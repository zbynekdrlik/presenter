# Changelog

## [0.1.2](https://github.com/zbynekdrlik/presenter/compare/v0.1.1...v0.1.2) (2026-01-29)


### Features

* AbleSet automation panel ([#34](https://github.com/zbynekdrlik/presenter/issues/34)) ([cecbcda](https://github.com/zbynekdrlik/presenter/commit/cecbcda3f842580ab2e50ce4b22e3682af06ef97))
* add curl one-liner install script ([6a1177c](https://github.com/zbynekdrlik/presenter/commit/6a1177c817a0f2bb0a5ba95b936ceef9fec8dba3))
* add timer overlay and stage layout controls ([#28](https://github.com/zbynekdrlik/presenter/issues/28)) ([3f94f3b](https://github.com/zbynekdrlik/presenter/commit/3f94f3bc201284f2f605e5063b304b8590554093))
* **ci:** add CI/CD workflows and SOTA versioning ([#79](https://github.com/zbynekdrlik/presenter/issues/79)) ([3dfee39](https://github.com/zbynekdrlik/presenter/commit/3dfee392a40480379ba870271805338d2b5cba25))
* **ci:** implement SOTA versioning and release strategy ([a1e60ab](https://github.com/zbynekdrlik/presenter/commit/a1e60abfebe9581bda56849a5b289929a3955d6e))
* complete presenter foundation ([#4](https://github.com/zbynekdrlik/presenter/issues/4)) ([9838c9a](https://github.com/zbynekdrlik/presenter/commit/9838c9ac3aba4e5d2e11cced410b3d9e98230118))
* cover live playlist removal and stage status ([#13](https://github.com/zbynekdrlik/presenter/issues/13)) ([6cbf5cf](https://github.com/zbynekdrlik/presenter/commit/6cbf5cf885238738db804098d0b77775855d9287))
* dockerize demo environments ([#9](https://github.com/zbynekdrlik/presenter/issues/9)) ([64cda39](https://github.com/zbynekdrlik/presenter/commit/64cda39b1ecb158b6c525804755f0c13c58a4efa))
* reduce trigger latency and stabilise demo tooling ([#25](https://github.com/zbynekdrlik/presenter/issues/25)) ([783e9f4](https://github.com/zbynekdrlik/presenter/commit/783e9f49bb5f6ce74c6400300fc57e9588e9989c))
* **stage:** add SNV scaler binary search algorithm for text fitting ([5c4b2d9](https://github.com/zbynekdrlik/presenter/commit/5c4b2d96defe6c2ec6a7f22540cfa41f64df49c8))
* tune operator UI and stage displays ([#12](https://github.com/zbynekdrlik/presenter/issues/12)) ([69ff09f](https://github.com/zbynekdrlik/presenter/commit/69ff09f7751dae16a57bad99cbe2b64f50a8aff3))
* **ui:** display version in operator UI footer ([da134b4](https://github.com/zbynekdrlik/presenter/commit/da134b4f00e9750e20c097c082c16f3de404ec6a))


### Bug Fixes

* **bible:** add bookCode and bookNumber to trigger endpoint ([c89f48f](https://github.com/zbynekdrlik/presenter/commit/c89f48f8185052260cdef1fbdea81ab96343f5af))
* **bible:** add missing /bible/books endpoint ([42b0b95](https://github.com/zbynekdrlik/presenter/commit/42b0b95ef02dd7d25f829b73a15ad9b989ef784c))
* **bible:** add missing /bible/resolve endpoint and compose_bible_slides ([ccc39dc](https://github.com/zbynekdrlik/presenter/commit/ccc39dc47f6546e8a92a3f47112158cf69417f6b))
* **bible:** add missing Bible presentations endpoints ([7cfce54](https://github.com/zbynekdrlik/presenter/commit/7cfce54ec0d3763d90b243a80d4b08080fbbfce9))
* **bible:** add PATCH endpoint and fix slide duplication ([aba351e](https://github.com/zbynekdrlik/presenter/commit/aba351e5fc50933010cedf36f00cd530c8a4bd86))
* **bible:** preserve original reference in trigger and fix test expectations ([e1aedc7](https://github.com/zbynekdrlik/presenter/commit/e1aedc7a08cf96f9493f6b2280b29d5b60a5bb95))
* **bible:** restore slide metadata and re-render modal on toggle ([6e2c826](https://github.com/zbynekdrlik/presenter/commit/6e2c82610a72c2c7419b7cdd99b1dab6f9e56e67))
* **bible:** support name and language updates in PATCH endpoint ([ba73970](https://github.com/zbynekdrlik/presenter/commit/ba73970f6fbfa08790ff0daf3ec710b42cdffad4))
* **bible:** sync Bible UI template with origin/main ([cc622a0](https://github.com/zbynekdrlik/presenter/commit/cc622a0b2994779a4152599e4592a1fd28a03107))
* **bible:** use passage range lookup for multi-verse slides ([8d4ebba](https://github.com/zbynekdrlik/presenter/commit/8d4ebba5d47bb5276b5b2548aca9a2e21bbca8da))
* **ci:** correct cargo-deny argument order ([644fe7b](https://github.com/zbynekdrlik/presenter/commit/644fe7b19ed1c00d58d4a004f746b99938c28940))
* **ci:** fix security workflow and enforce stricter CI policy ([4276145](https://github.com/zbynekdrlik/presenter/commit/4276145f86e471f559d8cac4163a9a9af572a3e0))
* **ci:** remove sccache for self-hosted runner compatibility ([8e307bf](https://github.com/zbynekdrlik/presenter/commit/8e307bf64c681e29e7ec1fcfaf9dd5b20ee0d5c1))
* **ci:** share cache between CI and E2E, add strict testing rules ([8291181](https://github.com/zbynekdrlik/presenter/commit/8291181ce3d557435243e489f60efddc04a626f2))
* **ci:** skip PR title validation for release PRs ([c08a8fd](https://github.com/zbynekdrlik/presenter/commit/c08a8fd6b5ff40f0d2b39d3f69a31bf75edf4d29))
* **ci:** switch PR Automation to self-hosted runner ([016285f](https://github.com/zbynekdrlik/presenter/commit/016285f8e30d4caa298947ba2d44d44e381bd441))
* **ci:** use dtolnay/rust-toolchain instead of rust-action ([d30edbf](https://github.com/zbynekdrlik/presenter/commit/d30edbfb1e259ba755746a6a898b67d82f20c525))
* **ci:** use rustsec/audit-check action for security audit ([#96](https://github.com/zbynekdrlik/presenter/issues/96)) ([9ca0c14](https://github.com/zbynekdrlik/presenter/commit/9ca0c14499115293e72b8f6f16f28e776f7bc7fc))
* **companion:** remove unwrap() calls from protocol.rs ([077775d](https://github.com/zbynekdrlik/presenter/commit/077775d362e8be767448226b993c6563b5735835))
* convert ProPresenter7-Proto from submodule to regular files ([d34c7d7](https://github.com/zbynekdrlik/presenter/commit/d34c7d759693457ffb63dff80fe16fbef9a3348a))
* **deploy:** add capability for binding to port 80 ([#98](https://github.com/zbynekdrlik/presenter/issues/98)) ([6d32d10](https://github.com/zbynekdrlik/presenter/commit/6d32d10484d728c86e19041731f20d5c7693484c))
* **deps:** pin base64ct to 1.6.0 for Rust 1.83 compatibility ([ee24288](https://github.com/zbynekdrlik/presenter/commit/ee24288efbd98b2af06d1ebd34d44cbfff94e87b))
* **deps:** update validator and fix cargo-deny config ([25d38d6](https://github.com/zbynekdrlik/presenter/commit/25d38d6b7656ea9fec33b0c4f35356df724bbaf2))
* **e2e:** increase timeout to 45 minutes for self-hosted runner ([a01e8bc](https://github.com/zbynekdrlik/presenter/commit/a01e8bcc576dd8de7b3ff39e0935c145ac5879a3))
* **e2e:** set PRESENTER_LIBRARY_ROOT for self-hosted runner ([33691b9](https://github.com/zbynekdrlik/presenter/commit/33691b93b11249473b50d075d603596ffe458ddd))
* **e2e:** simplify workflow to let tests manage their own servers ([f8c3f90](https://github.com/zbynekdrlik/presenter/commit/f8c3f909c66c1d6e70ea7ffa91a5cd0c561a5126))
* **e2e:** use testIgnore instead of test.skip to exclude demo tests in CI ([4112e20](https://github.com/zbynekdrlik/presenter/commit/4112e2005c4ef54d5773f9db73c44096e3879fac))
* **quality-check:** capture function-length checker JSON output and enforce violations; scope targets via QC_TARGETS to changed files only ([6231293](https://github.com/zbynekdrlik/presenter/commit/6231293db7018b8d755ac566e4c051c7266022ef))
* **quality:** remove expect() calls and add CI exemptions ([737864f](https://github.com/zbynekdrlik/presenter/commit/737864f31f6b18aee59da7dbc97a53909bfb712f))
* **quality:** resolve all clippy warnings and remove orphan test ([8ad998a](https://github.com/zbynekdrlik/presenter/commit/8ad998ae553c3a18cc2e8c00652b08a8ec0a5285))
* resolve merge compilation errors ([f263bb4](https://github.com/zbynekdrlik/presenter/commit/f263bb4abe69b3609d4c9db78babfcd0e87f94f6))
* restore library drag-to-playlist flow ([#21](https://github.com/zbynekdrlik/presenter/issues/21)) ([e9084bc](https://github.com/zbynekdrlik/presenter/commit/e9084bc0dc71822e9c6cbfaf28348f00d038998b))
* **security:** address code review findings and improve structure ([b5b61de](https://github.com/zbynekdrlik/presenter/commit/b5b61de657ff1d405f6bb1a8b9c205fa1c03d39f))
* **security:** prevent command injection in Android stage display ([#107](https://github.com/zbynekdrlik/presenter/issues/107)) ([08abe38](https://github.com/zbynekdrlik/presenter/commit/08abe3817285d9a97a4611ce354bdf1cc8bc2e44))
* **toolchain:** fix clippy and upgrade to stable Rust ([d681953](https://github.com/zbynekdrlik/presenter/commit/d681953fe1ecb966a726398c2fa7433f1e2cb9de))
* **toolchain:** upgrade to stable Rust for edition 2024 support ([4d31c7f](https://github.com/zbynekdrlik/presenter/commit/4d31c7ff8cab747f073870f1d0123aa8cc7779a5))


### Performance

* optimize latency-critical paths for live production ([ff9dbd8](https://github.com/zbynekdrlik/presenter/commit/ff9dbd859495ec6e68950d272cb203f56cfba9cc))


### Refactoring

* **persistence:** modularize repository and enforce 2025 structure ([#56](https://github.com/zbynekdrlik/presenter/issues/56)) ([ccbe6f1](https://github.com/zbynekdrlik/presenter/commit/ccbe6f1347f2a40511bcc36aef21e82600b6b413))
* **persistence:** split repository.rs into modules ([15d8174](https://github.com/zbynekdrlik/presenter/commit/15d81742913475c05650f71a9630d5b07cf92b35))
* simplify codebase structure and remove dead code ([b188b50](https://github.com/zbynekdrlik/presenter/commit/b188b509d4716e937ef9e9b72644b73245368980))
* SOTA 2025 project with GitHub Actions CI/CD ([97bdd22](https://github.com/zbynekdrlik/presenter/commit/97bdd223b8871a265fa5c16067a34ffac55bfd4a))
* **state:** split state.rs into focused modules ([fd9b53b](https://github.com/zbynekdrlik/presenter/commit/fd9b53bc14b858896379cf98227081b55067bb4e))


### Documentation

* add agent handbook ([#1](https://github.com/zbynekdrlik/presenter/issues/1)) ([7336490](https://github.com/zbynekdrlik/presenter/commit/7336490020e378b4c70acb7ac9613041a7b0759d))
* add state split follow-up issue body (create GH issue if CLI unavailable) ([c13448f](https://github.com/zbynekdrlik/presenter/commit/c13448f3dfaba90e983eea1069e042539f14e997))
* **claude:** add user preference for public IP URLs ([a70ae58](https://github.com/zbynekdrlik/presenter/commit/a70ae584b97d0e6b2e62bcd4fb3b226581c89f2d))
* document openspec project context ([#67](https://github.com/zbynekdrlik/presenter/issues/67)) ([6a2b359](https://github.com/zbynekdrlik/presenter/commit/6a2b35976e787c2330133eedf703828527dbc175))
* enforce tooling and runtime policy ([#3](https://github.com/zbynekdrlik/presenter/issues/3)) ([ae32fcc](https://github.com/zbynekdrlik/presenter/commit/ae32fcc6016c536370aa274560385de532fe1f98))


### Tests

* **e2e:** skip demo server tests in CI (requires Docker) ([c062faf](https://github.com/zbynekdrlik/presenter/commit/c062fafbeb57034ab4c7c8fc7b7651062363f8b3))


### CI/CD

* **release:** add release-please manifest config ([3d7ad06](https://github.com/zbynekdrlik/presenter/commit/3d7ad064bf144a0383b13dae9e3a3f1ac01ca2c3))
* **release:** add release-please manifest config ([43a3831](https://github.com/zbynekdrlik/presenter/commit/43a383116437a46c72f407885e817bb5ab2ca538))
* **version:** enforce version must be greater than latest release ([f38293d](https://github.com/zbynekdrlik/presenter/commit/f38293da30f27082ba6b9051f805ee1770212085))
