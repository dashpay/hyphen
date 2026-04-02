import { spawnSync } from "node:child_process";

function runPlaywright(reporter: "dot" | "line"): number {
  const result = spawnSync("pnpm", ["exec", "playwright", "test", "tests/play", `--reporter=${reporter}`], {
    cwd: new URL("../", import.meta.url),
    encoding: "utf8",
    stdio: reporter === "line" ? "inherit" : "pipe",
  });

  if (reporter === "dot" && result.stdout.length > 0) {
    process.stdout.write(result.stdout);
  }

  if (reporter === "dot" && result.stderr.length > 0) {
    process.stderr.write(result.stderr);
  }

  return result.status ?? 1;
}

const quickStatus = runPlaywright("dot");

if (quickStatus === 0) {
  process.stdout.write("PASS\n");
  process.exit(0);
}

process.stderr.write("FAIL\n");
process.stderr.write("Re-running with --reporter=line\n");

process.exit(runPlaywright("line"));
