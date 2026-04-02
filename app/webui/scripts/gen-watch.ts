import { spawn, type ChildProcess } from "node:child_process";

function run(
  command: string,
  args: readonly string[],
  onExit?: (code: number | null, signal: NodeJS.Signals | null) => void
): ChildProcess {
  const child = spawn(command, args, {
    cwd: process.cwd(),
    env: process.env,
    stdio: "inherit",
  });

  child.on("exit", (code, signal) => {
    onExit?.(code, signal);
  });

  child.on("error", (error) => {
    console.error(error);
    process.exitCode = 1;
  });

  return child;
}

let shuttingDown = false;
const children: ChildProcess[] = [];

function stopAll(signal: NodeJS.Signals): void {
  if (shuttingDown) {
    return;
  }

  shuttingDown = true;

  for (const child of children) {
    if (child.killed || child.exitCode != null) {
      continue;
    }

    child.kill(signal);
  }
}

function main(): void {
  const viewsWatch = run("pnpm", ["--filter", "@repo/hyphen-views", "gen:watch"], (code, signal) => {
    if (!shuttingDown && signal == null && code !== 0) {
      console.error(`views gen:watch exited with ${String(code)}`);
      stopAll("SIGTERM");
      process.exitCode = 1;
    }
  });
  const webuiWatch = run("pnpm", ["exec", "tsx", "./src/codegen/run.ts", "--watch"], (code, signal) => {
    if (!shuttingDown && signal == null && code !== 0) {
      console.error(`webui codegen watch exited with ${String(code)}`);
      stopAll("SIGTERM");
      process.exitCode = 1;
    }
  });

  children.push(viewsWatch, webuiWatch);
}

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    stopAll(signal);
    process.exit(0);
  });
}

main();
