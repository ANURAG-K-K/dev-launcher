# Multi-Repo Dev Launcher

A lightweight, native desktop app for discovering, configuring, and launching multiple
development repositories (microservices / independent repos) from one interface - with live
process tracking, launch profiles, and per-repo logs.

Built with **Tauri v2**, **Rust**, **React + Vite + TypeScript**, and **SQLite**.

> **Platform support:** Windows 10/11 only. The process manager relies on Windows Job Objects
> for tree-killing spawned processes, and the bundler targets NSIS - macOS/Linux are not
> currently supported.

## Features

- **Repository discovery** - scans a project root's immediate child folders for `package.json`
  and detects the package manager (pnpm / npm / yarn / bun).
- **Sequential launcher** - starts enabled repos one at a time with a configurable delay,
  respecting declared dependencies.
- **Launch profiles** - named sets of enabled repos, order, and command overrides.
- **Process tracking** - PID, status, exit code, and restart count per repo, with start / stop /
  restart / kill and a live log viewer (search, filter, auto-scroll).
- **Git integration** - per-repo branch status, pull / fetch, and branch switching with stash
  support.
- **Launch history** - every "Execute All" run is recorded with outcomes.

## Prerequisites

- [Node.js](https://nodejs.org/) 18+ and [pnpm](https://pnpm.io/)
- [Rust](https://www.rust-lang.org/tools/install) (stable toolchain)
- Windows 10/11 with [Tauri v2 system dependencies](https://v2.tauri.app/start/prerequisites/#windows)
  (WebView2, MSVC build tools)

## Getting started

```bash
pnpm install
pnpm tauri dev
```

## Building

```bash
pnpm tauri build
```

Produces an NSIS installer under `src-tauri/target/release/bundle/nsis/`.

## Project structure

```
src/            React + TypeScript frontend
src-tauri/      Rust backend (process manager, repo scanner, SQLite via sqlx)
src-tauri/migrations/   Database migrations
```

## License

All rights reserved.
