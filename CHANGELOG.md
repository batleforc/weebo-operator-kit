# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## [v0.18.1](https://github.com/batleforc/weebo-operator-kit/compare/c744f17e785bdc2186080bf691ec272144c3dba8..v0.18.1) - 2026-10-19
#### Features
- Merge branch 'fix/crd-docs-escape' - ([c4fe3b7](https://github.com/batleforc/weebo-operator-kit/commit/c4fe3b7629da28f63129dda133bd48faa68ba0ee)) - Max Batleforc
#### Bug Fixes
- (**ci**) operator-docs jobs install only node and task, mise authenticated (#8) - ([c744f17](https://github.com/batleforc/weebo-operator-kit/commit/c744f17e785bdc2186080bf691ec272144c3dba8)) - Maxime, Claude Opus 5.5 (1M context)
- (**operator-docs**) escape CRD text before it reaches MDX - ([f9b0729](https://github.com/batleforc/weebo-operator-kit/commit/f9b0729d02209aa4ebb368ffca9f5a422cd99834)) - Max Batleforc, Claude Opus 5.5 (1M context)

- - -

## [v0.18.0](https://github.com/batleforc/weebo-operator-kit/compare/c335330248add08cfcdd53fdde9e07b46ef90d98..v0.18.0) - 2026-10-18
#### Features
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) write only what differs — drift helpers, remote_writes_total, DriftCorrected; RFC 0001 moved here (#7) - ([08b2208](https://github.com/batleforc/weebo-operator-kit/commit/08b220873c72d01738c12a850564c9966b24f480)) - Maxime, Claude Opus 5.5 (1M context), Claude Opus 5.5 (1M context), Claude Opus 5.5 (1M context)
- setup wksa - ([3cbf319](https://github.com/batleforc/weebo-operator-kit/commit/3cbf31983b18af00920214651503f61a624d7877)) - Max Batleforc
- add devfile - ([2e6a942](https://github.com/batleforc/weebo-operator-kit/commit/2e6a942dc2e5ac229141a6dd9122d1aa43fa15cf)) - Max Batleforc
- ranged docs peers, NPM_CLIENT for kit:sync, KIT_TAG fallback - ([c335330](https://github.com/batleforc/weebo-operator-kit/commit/c335330248add08cfcdd53fdde9e07b46ef90d98)) - Max Batleforc, Claude Opus 5.5 (1M context)

- - -

## [v0.17.0](https://github.com/batleforc/weebo-operator-kit/compare/e23c8051aa947ad572e37e950245ea5c769134d4..v0.17.0) - 2026-10-16
#### Features
- managed workload core, LocalInstances and AttachNamespaces (v0.17.0) (#6) - ([e23c805](https://github.com/batleforc/weebo-operator-kit/commit/e23c8051aa947ad572e37e950245ea5c769134d4)) - Maxime, Claude Opus 5.5, Claude Opus 5.5, Claude Opus 5.5

- - -

## [v0.16.0](https://github.com/batleforc/weebo-operator-kit/compare/17205022d8f40d9e01e5600fb7245419613514cd..v0.16.0) - 2026-10-15
#### Features
- vault CAS claims, admission parity and shared operator helpers (review 2026-10-15) (#5) - ([1720502](https://github.com/batleforc/weebo-operator-kit/commit/17205022d8f40d9e01e5600fb7245419613514cd)) - Maxime, Claude Opus 5.5

- - -

## [v0.15.1](https://github.com/batleforc/weebo-operator-kit/compare/ccab8f9e860d7c956891ea75eeb0eaed452540b0..v0.15.1) - 2026-10-15
#### Bug Fixes
- (**publish**) a first prerelease publishes, marked as a prerelease (#4) - ([8d5a343](https://github.com/batleforc/weebo-operator-kit/commit/8d5a343569b08c77bd09f0544830401c595c8fae)) - Maxime, Claude Opus 5.5
#### Miscellaneous Chores
- (**deps**) rustls 0.23.46 (RUSTSEC-2026-0346..0349) (#3) - ([ccab8f9](https://github.com/batleforc/weebo-operator-kit/commit/ccab8f9e860d7c956891ea75eeb0eaed452540b0)) - Maxime, Claude Opus 5.5

- - -

## [v0.15.0](https://github.com/batleforc/weebo-operator-kit/compare/fd1fe206f7b6c0fa0dcf3c39d45513ad8e567256..v0.15.0) - 2026-10-14
#### Features
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) vault path grants on the cluster instance (#2) - ([fd1fe20](https://github.com/batleforc/weebo-operator-kit/commit/fd1fe206f7b6c0fa0dcf3c39d45513ad8e567256)) - Maxime, Claude Opus 5.5

- - -

## [v0.14.3](https://github.com/batleforc/weebo-operator-kit/compare/2daaade0adbb09f95f3997cccef91737b285523b..v0.14.3) - 2026-10-14
#### Bug Fixes
- (**template**) the operator's license is a placeholder (#1) - ([2daaade](https://github.com/batleforc/weebo-operator-kit/commit/2daaade0adbb09f95f3997cccef91737b285523b)) - Maxime, Claude Opus 5.5

- - -

## [v0.14.2](https://github.com/batleforc/weebo-operator-kit/compare/23c723dc53982954cd2283333a182fe3b6a26ee3..v0.14.2) - 2026-10-13
#### Bug Fixes
- (**template**) read a local instance's token through the opt-in check - ([23c723d](https://github.com/batleforc/weebo-operator-kit/commit/23c723dc53982954cd2283333a182fe3b6a26ee3)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.14.1](https://github.com/batleforc/weebo-operator-kit/compare/11301d55a15f14da6334f7525a41e279de7ff76a..v0.14.1) - 2026-10-13
#### Bug Fixes
- (**runtime**) lease takeover measured on the candidate's monotonic clock - ([11301d5](https://github.com/batleforc/weebo-operator-kit/commit/11301d55a15f14da6334f7525a41e279de7ff76a)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.14.0](https://github.com/batleforc/weebo-operator-kit/compare/ada19bee672aec918f8770a54dc85ab416c8f9ec..v0.14.0) - 2026-10-13
#### Bug Fixes
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) (**kit**) review fixes of 2026-10-13 in the crates - ([ada19be](https://github.com/batleforc/weebo-operator-kit/commit/ada19bee672aec918f8770a54dc85ab416c8f9ec)) - Max Batleforc, Claude Opus 5.5
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) (**tooling**) review fixes of 2026-10-13 in the chart, CI and template - ([cecf7ca](https://github.com/batleforc/weebo-operator-kit/commit/cecf7ca52eb99264a9f5d47f8932f8944825a486)) - Max Batleforc, Claude Opus 5.5
#### Documentation
- plan of the review fixes of 2026-10-13 - ([6a4c75c](https://github.com/batleforc/weebo-operator-kit/commit/6a4c75cd3961e0ac2e46db13147159303754aaf0)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.13.0](https://github.com/batleforc/weebo-operator-kit/compare/a06a448d7dc570ba1a965729a74d20cd33146ff2..v0.13.0) - 2026-10-13
#### Features
- (**publish**) amd64 by default, optional native arm64 images - ([a06a448](https://github.com/batleforc/weebo-operator-kit/commit/a06a448d7dc570ba1a965729a74d20cd33146ff2)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.12.1](https://github.com/batleforc/weebo-operator-kit/compare/ac0864eca55442b97c47634b7a6a316f6cf6cf71..v0.12.1) - 2026-10-13
#### Bug Fixes
- (**publish**) smoke-run each platform by its own manifest digest - ([ac0864e](https://github.com/batleforc/weebo-operator-kit/commit/ac0864eca55442b97c47634b7a6a316f6cf6cf71)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.12.0](https://github.com/batleforc/weebo-operator-kit/compare/882550805fe82b12b313c1def1a8e9c8795f8a68..v0.12.0) - 2026-10-13
#### Features
- (**template**) Hummingbird main image, static sibling, accepted Grype findings - ([8825508](https://github.com/batleforc/weebo-operator-kit/commit/882550805fe82b12b313c1def1a8e9c8795f8a68)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.11.0](https://github.com/batleforc/weebo-operator-kit/compare/682334ff81c4394c2545966fa08fd2b3430be99a..v0.11.0) - 2026-10-13
#### Features
- reusable publish workflow, Debian 13 and Hummingbird images - ([682334f](https://github.com/batleforc/weebo-operator-kit/commit/682334ff81c4394c2545966fa08fd2b3430be99a)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.10.1](https://github.com/batleforc/weebo-operator-kit/compare/247fa04c35c26b9841574c180f97978fbce9d3f5..v0.10.1) - 2026-10-12
#### Bug Fixes
- (**template**) kit:sync and kit:cog honour TAG - ([247fa04](https://github.com/batleforc/weebo-operator-kit/commit/247fa04c35c26b9841574c180f97978fbce9d3f5)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.10.0](https://github.com/batleforc/weebo-operator-kit/compare/82d8f66d0a2264320eaa9110cf21f788adb22e9a..v0.10.0) - 2026-10-12
#### Features
- ![BREAKING](https://img.shields.io/badge/BREAKING-red) watch scope, local instances and full namespaced mode - ([82d8f66](https://github.com/batleforc/weebo-operator-kit/commit/82d8f66d0a2264320eaa9110cf21f788adb22e9a)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.9.0](https://github.com/batleforc/weebo-operator-kit/compare/9718abdf885f89e4742a3f324ae281139eee0ba5..v0.9.0) - 2026-10-11
#### Features
- library chart, shared tasks, reusable CI and operator template - ([9718abd](https://github.com/batleforc/weebo-operator-kit/commit/9718abdf885f89e4742a3f324ae281139eee0ba5)) - Max Batleforc

- - -

## [v0.8.0](https://github.com/batleforc/weebo-operator-kit/compare/003f8b64886bb4c85a15608e43b9e5c83219bbaa..v0.8.0) - 2026-10-11
#### Features
- (**perf**) measure the resource budget every week - ([003f8b6](https://github.com/batleforc/weebo-operator-kit/commit/003f8b64886bb4c85a15608e43b9e5c83219bbaa)) - Max Batleforc, Claude Opus 5.5

- - -

## [v0.7.0](https://github.com/batleforc/weebo-operator-kit/compare/55d2fd8516efdca1f084464fdd0d0267a258a78c..v0.7.0) - 2026-10-11
#### Features
- shared docs-site package and cog.toml template for the operators - ([55d2fd8](https://github.com/batleforc/weebo-operator-kit/commit/55d2fd8516efdca1f084464fdd0d0267a258a78c)) - Max Batleforc, Claude Opus 5.5

- - -

Changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto).