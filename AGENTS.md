Contract notes for this repo:

- keep is a Rust CLI, one Cloudflare Worker and one R2 bucket. Keep it that way: no accounts and no database. Link previews show public file details or an encrypted-file card.
- Files are permanent until deleted with their delete token, unless uploaded with `--expires` (`X-Expires`). Expired files stop being served at once and an hourly cron deletes them from R2.
- The CLI surface is `keep [file]`, `keep get`, `keep ls`, `keep rm` and `keep config`, with `-e`, `-n`, `--expires` and `-o`. Explicit files take priority over piped input, then the clipboard. Terminal uploads copy links by default; captured stdout stays links only and never changes the clipboard. Add a flag only when a workflow cannot be done without it.
- Encryption happens on the client: AES-256-GCM, stored as nonce, ciphertext, tag, with the base64url key in the link fragment. Encrypted uploads seal the filename the same way, base64url-encoded in `X-Encrypted-Name`, and store `encrypted` as the visible name. The server never sees keys, and the CLI and the download page must stay compatible.
- Uploads respond with the link as text plus `X-Delete-Token`, so `curl -T` stays useful.
- Pages are monospace, monotone and framework-free, with system fonts only.
- Tests are end-to-end in `test/e2e.test.mjs`. Cover user workflows, not internals.
