# Changelog

## 0.7.0 · 2026-10-18

Large upload histories are faster to list and update. Uploads use less temporary memory, and downloads reuse buffers and encryption keys. Existing links, encrypted files, retries, clipboard preferences and command output remain compatible.

### Measured change after merging the performance work

The baseline is [main immediately before this merge batch](https://github.com/Stoffbergie/keep/commit/8437886c32ca32bc659d9c986db28a351a10f638). The after measurements use the combined source released in 0.7.0. These are the effective changes from the full batch, rather than the sum of individual PR estimates.

| Workflow and measurement | Before | 0.7.0 | Change |
| --- | ---: | ---: | ---: |
| List a 100,000-entry history: elapsed time | 69.1 ms | 54.3 ms | 21% less time |
| Remove the final entry from that history: elapsed time | 84.3 ms | 65.9 ms | 22% less time |
| List that history: peak Rust heap | 65.1 MiB | 36.4 MiB | 44% lower |
| List that history: allocation count | 1,360,239 | 540,152 | 60% fewer |
| Upload an 8 MiB plain file: peak Rust heap | 3.13 MiB | 2.13 MiB | 32% lower |
| Upload an 8 MiB encrypted file: cumulative allocated bytes | 25.20 MiB | 8.19 MiB | 67% lower |
| Upload 64 MiB + 5 bytes of encrypted piped input: peak Rust heap | 75.15 MiB | 40.15 MiB | 47% lower |
| Browser download with an encrypted filename: key imports | 3 | 1 | 67% fewer |

Measurements were taken on an Apple M5 Pro with 48 GiB RAM, macOS 27.2 and Rust 1.98.1, using optimized builds and local HTTP fixtures. History timings are medians of 12 alternating before/after pairs after two warm-up pairs, with allocation counters disabled. Heap and allocation figures are medians of six alternating pairs in separate instrumented builds; they exclude operating-system buffers. Cumulative allocation measures all bytes requested over the operation, not peak memory held at once. History and configuration paths were redirected to disposable fixtures.

Local upload and download elapsed times were broadly unchanged or varied between runs. These results do not establish faster internet transfers or lower peak memory for every workload. Browser key reuse was checked with both an 8 MiB streaming-format file and a legacy-format file, including encrypted filename recovery and matching downloaded bytes.

### Compatibility checks

The existing end-to-end suite passed on the merged source. Additional local round trips checked file and piped uploads, plain and encrypted downloads, multipart retries, original and streaming encryption formats, and history output. Worker HEAD responses retained their status and headers with empty bodies while avoiding unnecessary page and preview work.
