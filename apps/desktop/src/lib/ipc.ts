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

export function errorCode(e: unknown): string | null {
  return e && typeof e === "object" && "code" in e ? String((e as CoreError).code) : null;
}

// ---------- terminals ----------

export type ShellRequest =
  | { kind: "power_shell" }
  | { kind: "wsl"; distro: string | null }
  | { kind: "ssh"; host_id: string };

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

// ---------- hosts ----------

export interface SshView {
  address: string;
  port: number;
  username: string;
  private_key_path: string;
  fingerprint: string | null;
  has_passphrase: boolean;
}

export interface HostView {
  id: string;
  display_name: string;
  os_family: string;
  is_local: boolean;
  trust_state: string;
  last_seen_at: string | null;
  capabilities: {
    os_pretty_name?: string | null;
    arch?: string;
    adb_path?: string | null;
  } | null;
  connected: boolean;
  ssh: SshView | null;
}

export interface HostInput {
  display_name: string;
  os_family: string;
  address: string;
  port: number;
  username: string;
  private_key_path: string;
}

export interface HostKeyCheck {
  presented: string;
  pinned: string | null;
  matches: boolean;
}

export const hostsList = () => invoke<HostView[]>("hosts_list");
export const hostCreate = (input: HostInput) => invoke<HostView>("host_create", { input });
export const hostUpdate = (id: string, input: HostInput) => invoke<HostView>("host_update", { id, input });
export const hostDelete = (id: string) => invoke<void>("host_delete", { id });
export const hostCheckKey = (id: string) => invoke<HostKeyCheck>("host_check_key", { id });
export const hostTrustKey = (id: string, fingerprint: string) =>
  invoke<HostView>("host_trust_key", { id, fingerprint });
export const hostSetPassphrase = (id: string, passphrase: string | null) =>
  invoke<HostView>("host_set_passphrase", { id, passphrase });
export const hostConnect = (id: string) => invoke<HostView>("host_connect", { id });
export const hostDisconnect = (id: string) => invoke<HostView>("host_disconnect", { id });

// ---------- files ----------

export interface RemoteEntry {
  name: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  modified: number | null;
}

export interface Listing {
  path: string;
  entries: RemoteEntry[];
}

export interface TransferEvent {
  id: number;
  state: "running" | "done" | "failed" | "cancelled";
  done: number;
  total: number;
  result_path: string | null;
  error: CoreError | null;
}

export const sftpList = (hostId: string, path: string) => invoke<Listing>("sftp_list", { hostId, path });

export function sftpDownload(
  hostId: string,
  remotePath: string,
  localDir: string,
  overwrite: boolean,
  onEvent: (e: TransferEvent) => void,
): Promise<number> {
  return invoke<number>("sftp_download", {
    hostId,
    remotePath,
    localDir,
    overwrite,
    onEvent: new Channel<TransferEvent>(onEvent),
  });
}

export function sftpUpload(
  hostId: string,
  localPath: string,
  remoteDir: string,
  overwrite: boolean,
  onEvent: (e: TransferEvent) => void,
): Promise<number> {
  return invoke<number>("sftp_upload", {
    hostId,
    localPath,
    remoteDir,
    overwrite,
    onEvent: new Channel<TransferEvent>(onEvent),
  });
}

export const transferCancel = (id: number) => invoke<void>("transfer_cancel", { id });

// ---------- android devices ----------

export interface DeviceView {
  device_id: string;
  display_name: string;
  manufacturer: string | null;
  model: string | null;
  identity_status: string;
  route_id: string;
  host_id: string;
  host_name: string;
  connection_type: "usb_adb" | "wifi_adb";
  serial: string;
  state: "available" | "unauthorized" | "offline" | "unavailable";
  last_seen_at: string | null;
  transport_id: number | null;
}

export interface WatchStatus {
  watching: boolean;
  detail: string;
}

export interface AdbStatus {
  local_adb_path: string | null;
  hosts: Record<string, WatchStatus>;
}

export interface DeviceInfo {
  route_id: string;
  host_name: string;
  serial: string;
  transport_id: number;
  props: Array<[string, string]>;
}

export interface ShellResult {
  route_id: string;
  host_name: string;
  serial: string;
  output: string;
  truncated: boolean;
}

export const devicesList = () => invoke<DeviceView[]>("devices_list");
export const adbStatus = () => invoke<AdbStatus>("adb_status");
export const deviceInfo = (routeId: string) => invoke<DeviceInfo>("device_info", { routeId });
export const deviceShell = (routeId: string, expectedSerial: string, command: string) =>
  invoke<ShellResult>("device_shell", { routeId, expectedSerial, command });
export const deviceRename = (deviceId: string, name: string) => invoke<void>("device_rename", { deviceId, name });
export const deviceForget = (routeId: string) => invoke<void>("device_forget", { routeId });

// ---------- android tools ----------

export interface DeviceEntry {
  name: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  mtime: number;
}

export const deviceScreenshot = (routeId: string) => invoke<ArrayBuffer>("device_screenshot", { routeId });
export const deviceScreenshotSave = (routeId: string, path: string) =>
  invoke<void>("device_screenshot_save", { routeId, path });

export function logcatStart(routeId: string, onData: (bytes: Uint8Array) => void): Promise<number> {
  const channel = new Channel<ArrayBuffer>((buf) => onData(new Uint8Array(buf)));
  return invoke<number>("logcat_start", { routeId, onData: channel });
}
export const logcatStop = (id: number) => invoke<void>("logcat_stop", { id });

export const devicePackages = (routeId: string) => invoke<string[]>("device_packages", { routeId });
export const deviceUninstall = (routeId: string, expectedSerial: string, pkg: string) =>
  invoke<string>("device_uninstall", { routeId, expectedSerial, package: pkg });
export const deviceInstallApk = (
  routeId: string,
  expectedSerial: string,
  localPath: string,
  onEvent: (e: TransferEvent) => void,
) => invoke<number>("device_install_apk", { routeId, expectedSerial, localPath, onEvent: new Channel<TransferEvent>(onEvent) });

export const deviceFilesList = (routeId: string, path: string) => invoke<DeviceEntry[]>("device_files_list", { routeId, path });
export const devicePush = (
  routeId: string,
  expectedSerial: string,
  localPath: string,
  remoteDir: string,
  overwrite: boolean,
  onEvent: (e: TransferEvent) => void,
) =>
  invoke<number>("device_push", {
    routeId,
    expectedSerial,
    localPath,
    remoteDir,
    overwrite,
    onEvent: new Channel<TransferEvent>(onEvent),
  });
export const devicePull = (
  routeId: string,
  remotePath: string,
  localDir: string,
  overwrite: boolean,
  onEvent: (e: TransferEvent) => void,
) => invoke<number>("device_pull", { routeId, remotePath, localDir, overwrite, onEvent: new Channel<TransferEvent>(onEvent) });
