# Changelog

## 0.2.0

### Changed
- Renamed the app to keep. The CLI is `keep` and the service is `keep.stoff.dev`.
- Moved files into the `keep-files` bucket. Only `keep.stoff.dev` is supported.
- Files never expire. Expiry settings, burn-after-read, custom names and share modes are gone.
- The CLI is down to `keep <file>`, `keep get`, `keep ls` and `keep rm`, with `-e` to encrypt and `-n` to name stdin.
- Large uploads send four parts at a time. Downloads stream to disk and only appear under their final name once complete.
- Uploads work with plain `curl -T`, and the response is the link itself.
- Every file lives in R2 with its details in the object metadata. No database, KV or scheduled cleanup. All existing uploads were moved into R2.

### Fixed
- Existing files show their original upload dates on their download pages.
- Large uploads return their delete token, so they can be removed with `keep rm`.

### Added
- A minimal landing page, and a download page for each file that decrypts encrypted links in the browser.
- Search metadata and link previews for the homepage and shared files, including image thumbnails, text excerpts and encrypted-file cards.

### Removed
- The TypeScript SDK, the `/api/v1` routes, the metadata route, usage metrics, idempotency keys, clipboard support, history search and the config command.

## 0.1.13 - 2026-06-10

### Changed
- Focused the repo on CLI/API file sharing: upload, share, download, and delete by token.
- Made one-year expiry the default, with `temporary`, `private`, and `permanent` modes for explicit overrides.
- Share URLs now serve file bytes directly instead of HTML previews.
- Root lint and CI now run real TypeScript and Rust checks.
- Release metadata now uses one version across the workspace and CLI crate.
- Deploy now uses a production smoke check instead of brittle live latency assertions.
- Release packaging now validates archive contents and runnable CLI artifacts before publishing.
- Fresh installs use pnpm 10 with explicit dependency build-script policy.

### Added
- Multipart upload support for large files.
- API compatibility, e2e, integration, and unit test lanes.
- SDK helpers for push, upload, download, and pull.

### Removed
- Web app workspace and browser renderer.
- Better Auth, account tables, API keys, and collection scaffolding.
