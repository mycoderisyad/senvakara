// Typed wrappers over Tauri commands. Errors arrive as the core's
// structured `CoreError` ({ code, message }); see docs/API.md §3.
import { Channel, invoke } from "@tauri-apps/api/core";

export interface CoreError {
  code: string;
  message: string;
}

export function errorText(e: unknown): string {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    const ce = e as CoreError;
    return `${ce.code}: ${ce.message}`;
  }
  return String(e);
}

export type ShellRequest = { kind: "power_shell" } | { kind: "wsl"; distro: string | null };

export function termOpen(
  shell: ShellRequest,
  cols: number,
  rows: number,
  onData: (bytes: Uint8Array) => void,
): Promise<number> {
  const channel = new Channel<ArrayBuffer>((buf) => onData(new Uint8Array(buf)));
  return invoke<number>("term_open", { shell, cols, rows, onData: channel });
}

export const termWrite = (id: number, data: string) => invoke<void>("term_write", { id, data });
export const termResize = (id: number, cols: number, rows: number) =>
  invoke<void>("term_resize", { id, cols, rows });
export const termClose = (id: number) => invoke<void>("term_close", { id });
export const wslDistros = () => invoke<string[]>("wsl_distros");
