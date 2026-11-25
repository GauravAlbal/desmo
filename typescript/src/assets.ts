import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export function packageRoot(): string {
  return join(dirname(fileURLToPath(import.meta.url)), "..");
}

export function bundledMachine(): string {
  return join(packageRoot(), "assets", "verification_attempt.ik.toml");
}

export function bundledOrderMachine(): string {
  return join(packageRoot(), "assets", "order.ik.toml");
}
