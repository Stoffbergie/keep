import { spawnSync } from "node:child_process"

const transfers = "large files|large piped output|medium files|multipart boundaries"

for (const filter of [`--test-name-pattern=${transfers}`, `--test-skip-pattern=${transfers}`]) {
  const result = spawnSync(process.execPath, ["--test", filter, "test/e2e.test.mjs"], {
    stdio: "inherit",
  })
  if (result.status !== 0) process.exit(result.status ?? 1)
}
