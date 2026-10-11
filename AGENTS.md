Contract notes for this repo:

- keep is a Rust CLI, one Cloudflare Worker and one R2 bucket. Keep it that way: no accounts, no expiry and no database. Link previews show public file details or an encrypted-file card.
- Files are permanent until deleted with their delete token.
- The CLI surface is `keep [file]`, `keep get`, `keep ls` and `keep rm`, with `-e`, `-n` and `-o`. Add a flag only when a workflow cannot be done without it.
- Encryption happens on the client: AES-256-GCM, stored as nonce, ciphertext, tag, with the base64url key in the link fragment. The server never sees keys, and the CLI and the download page must stay compatible.
- Uploads respond with the link as text plus `X-Delete-Token`, so `curl -T` stays useful.
- Pages are monospace, monotone and framework-free, with system fonts only.
- Tests are end-to-end in `test/e2e.test.mjs`. Cover user workflows, not internals.
