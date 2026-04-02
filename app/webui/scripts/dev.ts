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

async function main(): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const child = run("pnpm", ["gen"], (code, signal) => {
      if (signal != null || code !== 0) {
        reject(new Error(`pnpm gen exited with ${signal ?? String(code)}`));
        return;
      }

      resolve();
    });
    children.push(child);
  });

  await new Promise<void>((resolve, reject) => {
    const child = run("pnpm", ["exec", "svelte-kit", "sync"], (code, signal) => {
      if (signal != null || code !== 0) {
        reject(new Error(`svelte-kit sync exited with ${signal ?? String(code)}`));
        return;
      }

      resolve();
    });
    children.push(child);
  });

  const genWatch = run("pnpm", ["gen:watch"], (code, signal) => {
    if (!shuttingDown && signal == null && code !== 0) {
      console.error(`pnpm gen:watch exited with ${String(code)}`);
      stopAll("SIGTERM");
      process.exitCode = 1;
    }
  });
  const vite = run("pnpm", ["exec", "vite", "dev"], (code, signal) => {
    if (!shuttingDown && signal == null && code !== 0) {
      console.error(`vite dev exited with ${String(code)}`);
      stopAll("SIGTERM");
      process.exitCode = 1;
    }
  });

  children.push(genWatch, vite);
}

for (const signal of ["SIGINT", "SIGTERM"] as const) {
  process.on(signal, () => {
    stopAll(signal);
    process.exit(0);
  });
}

void main().catch((error: unknown) => {
  console.error(error);
  stopAll("SIGTERM");
  process.exit(1);
});
