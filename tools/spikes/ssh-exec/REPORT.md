# Spike WF Fase 0 #2 — eksekusi script remote via russh

**Keputusan: GO.** Kelima pertanyaan terjawab positif di Ubuntu 24.04 (WSL2) dengan russh =0.65.2. Ada empat aturan desain yang wajib diikuti; detailnya di bagian "Wajib di produksi" di bawah.

- Tanggal: 2026-11-27
- Host uji: WSL `Ubuntu-24.04`. Kernel 6.6.87.2-microsoft-standard-WSL2, Sudo 1.9.15p5, OpenSSH_9.6p1 Ubuntu-3ubuntu13.20. User `svtest` dengan sudo yang tetap minta password, sshd di 127.0.0.1:2222.
- **Debian 12/13 dan Ubuntu 22.04 belum diverifikasi** karena belum ada host.
- Kode: [src/main.rs](src/main.rs). Crate standalone dengan `[workspace]` sendiri, bergantung ke `senvakara-adapters-ssh` lewat path, plus `russh =0.65.2` dan `russh-sftp =3.0.1`.
- Runtime tokio dibuat **current_thread** dengan ticker tiap 50 ms. Kalau ada yang nge-block runtime, jumlah tick akan tertinggal.

## Cara reproduksi

```powershell
# 1) host uji (owner, sebagai root di WSL; kredensial random ditulis ke creds.local, gitignored)
wsl -d Ubuntu-24.04 -u root -- bash /mnt/d/LAIN-LAIN/code/senvakara/tools/spikes/ssh-exec/setup-wsl.sh
# 2) spike (~60 s). Log lengkap: runs/out.txt (gitignored)
cd tools/spikes/ssh-exec; cargo run -q > runs/out.txt
# 3) bongkar host uji
wsl -d Ubuntu-24.04 -u root -- bash /mnt/d/LAIN-LAIN/code/senvakara/tools/spikes/ssh-exec/teardown-wsl.sh
```

Spike dijalankan 2x dengan hasil yang sama. Run kedua menambah tes 1e. Ukuran `target/` ±400 MB, disk D: sisa ±20 GB.

## 1. `sudo -S` + password via stdin + EOF

**Jawaban: bisa.** Caranya: tulis `password\n` ke channel, lalu panggil `ChannelWriteHalf::eof()`. Script melihat stdin EOF.

Yang dijalankan: `sudo -k -S -p '' -- bash -euo pipefail <file>`. Argv di-quote dengan `shell_quote` dari adapter. Password tidak pernah masuk argv.

```
[1a] password+EOF   exit_status=Some(0) eof=true close=true
    stdout| uid=0 user=root
    stdout| stdin=eof
    stdout| tty=not a tty
[1c] password salah exit_status=Some(1), stdout kosong (script tidak jalan), 2.2–2.7 s
    stderr| Sorry, try again. / sudo: no password was provided / sudo: 1 incorrect password attempt
3a: password present in `ps -eo args` output: false
```

**Temuan penting (1b):** sudo hanya membaca satu baris password. Sisa stdin **diteruskan ke script**: `password\nhello\n` menghasilkan `stdin-line-len=5`. Artinya, kalau sudo tidak meminta password (NOPASSWD, atau timestamp masih ter-cache), **password akan jadi stdin script**.

- Perbaikan: script dijalankan dengan `</dev/null`.
- Tes 1e: launcher produksi plus password plus baris ekstra menghasilkan `uid=0` dan `stdin=eof`.
- Launcher yang sama tanpa sudo juga menghasilkan `stdin=eof`.

**Soal `SshPtyWriter::close()`: itu CHANNEL_CLOSE, bukan EOF.**
- Bukti dari source: `ChannelWriteHalf::close()` mengirim `ChannelMsg::Close` (`russh-0.65.2/src/channels/mod.rs:416`). Di sisi session, ini jadi `CHANNEL_CLOSE` dan channel langsung dihapus dari map lokal (`src/session.rs:287-295`). Untuk EOF ada method terpisah, `eof()`, yang mengirim `CHANNEL_EOF` (`channels/mod.rs:407`, `session.rs:278`).
- Bukti dari tes 1d (`spawn()`, lalu `write(pw)`, lalu `close()`): reader **tidak menerima event apa pun dalam 5 s** (`"no event within 5 s"`). Exit status hilang dan `SshPtyReader::next()` tidak mengembalikan `None`, jadi menggantung. Padahal script tetap jalan sebagai root (`marker exists, owner root`).
- Yang dibutuhkan: method `eof()` terpisah di `SshPtyWriter`. Isinya cukup satu baris, `self.0.eof().await`. Untuk workflow, lebih baik pakai `exec_stream` baru, bukan `spawn`.

## 2. stdout vs stderr terpisah + exit status

**Jawaban: bisa.** `ChannelMsg::Data` berisi stdout, `ChannelMsg::ExtendedData { ext: 1 }` berisi stderr. Tidak ada nilai `ext` selain 1. Urutan antar-chunk tetap terjaga.

```
[2 plain]     exit_status=Some(7)  chunk order: [stdout 6, stderr 6, stdout 6, stderr 6]
[2 via sudo]  exit_status=Some(7)  chunk order: [stdout 6, stderr 6, stdout 6, stderr 6]
```

Catatan: `SshPtyReader` yang sekarang menggabungkan keduanya (`pty.rs:42`). `exec` sudah memisahkan, tapi hasilnya di-buffer.

## 3. Cancel via `setsid` + pidfile + kill process group

**Jawaban: bisa, dan ini satu-satunya cara yang benar-benar membersihkan semua proses.**

Launcher yang dipakai (argv, di-quote):

```
sudo -k -S -p '' -- setsid -w bash -c 'echo $$ > "$1"; exec bash -euo pipefail "$2" </dev/null' sv-run <pidfile> <script>
```

Cancel dijalankan lewat exec kedua di koneksi yang sama:

```
sudo -k -S -p '' -- bash -c <KILL_GROUP> sv-kill <pidfile>
```

`KILL_GROUP` bekerja seperti ini:
1. Memvalidasi pgid: harus angka dan lebih dari 1. Tanpa validasi ini, `kill -- -1` bisa mengenai semua proses.
2. Mengirim `TERM` ke grup.
3. Menunggu sampai 3 s.
4. Kalau grup masih ada, mengirim `KILL`.

Script uji meniru apt: loop di foreground, `sleep 1000 &`, plus subshell yang mengabaikan TERM.

| Varian | Hasil |
|---|---|
| **3a** kill pgid | Sebelum: 5 proses root di pgid=sid=19006. Kill: `killed with KILL after TERM grace` (3.4 s). Channel utama: `exit_signal=Some("TERM") eof=true close=true`, 6.5 s. Sesudah: `(no processes in group)` |
| **3b** hanya `close()` channel | Bash utama mati karena SIGPIPE. **`sleep 1000` dan subshell tetap hidup** sampai di-kill manual. Channel tidak menerima Eof/Close (sudah dihapus secara lokal), drain habis 10 s. |
| **3c** SSH `signal(Sig::TERM)` | Terkirim (`Ok(())`, `exit_signal=TERM`), tapi hanya top process yang mati. **Child yatim tetap hidup dan tetap memegang pipe stdout**, jadi channel tidak pernah Eof/Close (`eof=false close=false`). |

Setelah semua run, tidak ada proses yang tersisa di host (`ps` tidak menemukan `senvakara` atau `sleep 1000`).

## 4. Timeout + output cap tanpa nge-block runtime

**Jawaban: bisa.** Skenario uji: tanpa sudo, script mengeluarkan 5 MB lalu `sleep 30`, dengan timeout 5 s dan cap 2 MB per stream.

```
[4 main channel] exit_status=Some(15) timed_out=true truncated=true stdout_total=5000001 elapsed=5.07s
  stdout kept=2097152 bytes (cap 2097152), 5 MB diterima dalam 0.06–0.08 s
  kill exec (no sudo): group gone after TERM (1x100ms)
  runtime ticker: 101 ticks of 50 ms in 5.07s (expected ~101)
== total 59.0s, ticker 1179 ticks (expected ~1179)
```

- Kelebihan di atas cap tetap dibaca, dihitung, lalu dibuang. Channel tetap dikuras supaya window SSH tidak macet.
- Semua penantian memakai `tokio::select!` atau `timeout_at`, dan sepanjang run tidak ada satu tick pun yang hilang.
- Exit code setelah stop **tidak konsisten**:
  - Lewat sudo: `exit_signal=TERM`.
  - Tanpa sudo: `exit_status=15`, ditambah stderr `setsid: child … did not exit normally` (dari `setsid -w`).
  - Jadi hasil akhir run harus ditentukan dari alasan stop yang dicatat app sendiri (timeout atau cancel), bukan dari exit code.
- Belum diuji: consumer yang lambat (backpressure ke UI) dan output yang terus mengalir sampai tepat di batas timeout.

## 5. SFTP upload ke `~/.cache/senvakara/run-<id>/` (0700) + cleanup

**Jawaban: bisa.** Langkah-langkahnya:
1. `canonicalize(".")` untuk mendapatkan path home.
2. `create_dir` untuk folder yang belum ada, lalu `sftp::chmod(…, 0o700)` dari adapter pada `senvakara/` dan `run-<id>/`.
3. File di-upload dengan `open_with_flags_and_attributes(CREATE|EXCLUDE|WRITE, permissions=0o700)`. Mode langsung di-set saat open, jadi tidak ada jendela waktu di mana file sempat world-readable.

```
700 svtest:svtest /home/svtest/.cache/senvakara
700 svtest:svtest /home/svtest/.cache/senvakara/run-20768
700 svtest:svtest .../run-20768/s1.sh   (begitu juga s1close.sh, s2.sh, s3.sh, s4.sh)
re-upload same path with CREATE|EXCLUDE -> error (Failure)   ← tidak menimpa / tidak mengikuti file yang sudah ada
sftp::remove -> Ok(())  → "gone"
```

- Cleanup memakai `sftp::remove` yang sudah ada di adapter. Cara ini berhasil walaupun di dalam folder ada file milik root (`pgid`, `ran-close`), karena yang dicek untuk menghapus adalah izin tulis pada folder.
- Batasnya: kalau script root membuat **subfolder** berisi file di dalam run dir, user biasa tidak bisa menghapusnya. Aturannya: script tidak boleh menulis ke run dir. Kalau tetap perlu, cleanup harus pakai `sudo rm -rf -- <dir>` (path tervalidasi).

## Wajib di produksi (hasil spike)

1. **Selalu kirim EOF setelah stdin** dengan `eof()`, jangan `close()`. `close()` membuang exit status dan membuat reader menggantung.
2. **Stdin script harus `</dev/null`** di dalam launcher, ditambah `sudo -k`. Tanpa ini, password bisa bocor ke stdin script ketika sudo tidak meminta password.
3. **Cancel dan timeout harus kill process group** lewat exec kedua (TERM, tunggu, lalu KILL), dengan validasi pgid. Menutup channel atau mengirim SSH `signal` saja meninggalkan proses yatim, dan channel tidak akan pernah close.
4. **Drain dibatasi waktu** setelah stop (spike memakai 10 s). Setelah itu channel ditutup dan status diset ke `cancelled` / `timed_out`, bukan dari exit code.
5. Pidfile dan run dir harus unik per node per run. Launcher sebaiknya gagal kalau pidfile tidak bisa ditulis; di tes 1e versi tanpa sudo, launcher lanjut jalan walaupun tulis pidfile ditolak.
6. Untuk mendeteksi password sudo salah: hasilnya exit 1 tanpa stdout dan dengan stderr dari sudo, dan bisa makan ±2.5 s karena delay bawaan sudo. Pesannya bergantung locale, jadi lebih baik pre-check sekali per run dengan `sudo -k -S -p '' -- true` (exit 0 berarti password benar) sebelum apply. Pre-check ini **belum diuji terpisah**; perilakunya sama dengan 1a/1c.

## Usulan API (bentuk minimal)

```rust
// crates/adapters-ssh/src/exec.rs (baru)
pub enum ExecEvent { Stdout(Vec<u8>), Stderr(Vec<u8>) }

pub enum ExecEnd {
    Exited(u32),          // ExitStatus
    Signalled(String),    // ExitSignal
    TimedOut,             // deadline lewat, stopper sudah dipanggil
    Cancelled,            // cancel di-set, stopper sudah dipanggil
    Lost,                 // channel/koneksi hilang tanpa status → UI: "unknown"
}

pub struct ExecReport {
    pub end: ExecEnd,
    pub stdout_total: u64,
    pub stderr_total: u64,
    pub truncated: bool,     // ada stream yang melewati `cap`
}

impl SshConnection {
    /// Exec argv (POSIX-quoted), tulis `stdin` lalu EOF (selalu EOF, juga saat
    /// stdin None), stream stdout/stderr terpisah ke `events`, simpan maks `cap`
    /// byte per stream di sisi pemanggil, berhenti saat `timeout`/`cancel`;
    /// saat berhenti panggil `stop` (mis. kill pgid via exec kedua) lalu drain
    /// maksimal `drain` sebelum menutup channel.
    pub async fn exec_stream(
        &self,
        argv: &[&str],
        stdin: Option<Secret>,                    // Zeroizing; tidak pernah di-log
        timeout: Duration,
        cap: usize,
        cancel: tokio::sync::watch::Receiver<bool>,
        events: tokio::sync::mpsc::Sender<ExecEvent>,
        stop: impl FnOnce() -> BoxFuture<'static, ()> + Send,
    ) -> Result<ExecReport>;
}

// crates/adapters-ssh/src/sftp.rs
/// Buat file baru eksklusif (CREATE|EXCLUDE|WRITE) dengan `mode` di-set saat
/// open; gagal bila path sudah ada.
pub async fn upload_bytes(sftp: &SftpSession, path: &str, bytes: &[u8], mode: u32) -> Result<()>;
/// mkdir -p dengan chmod `mode` untuk tiap level yang dibuat (dan `leaf`).
pub async fn ensure_dir(sftp: &SftpSession, path: &str, mode: u32) -> Result<()>;
```

Catatan untuk API ini:
- Cap ditegakkan di sisi pemanggil. Spike menyimpan potongan di memori; di produksi isinya ke file log `runs/<run_id>/<node>.log`, dan UI menampilkan 500 baris terakhir.
- Launcher sudo dan `KILL_GROUP` ditempatkan di `crates/app` sebagai string konstanta yang di-review, karena keduanya bukan urusan adapter.
- `Bytes` di dalam russh menyimpan salinan password yang tidak di-zeroize. Ini batasan library dan dicatat di ADR-029.

## Belum diverifikasi

- Debian 12/13 dan Ubuntu 22.04. Yang perlu dicek di sana: `use_pty` di sudoers Debian, perilaku sudo versi lain, dan `setsid -w` dari util-linux versi lain.
- VPS sungguhan lewat jaringan dengan latensi, dan koneksi yang putus di tengah apply.
- Consumer yang lambat atau backpressure.
- Host dengan NOPASSWD sudo. Mekanisme pengamannya (`</dev/null`) sudah terbukti di tes 1e tanpa sudo, tapi sudoers NOPASSWD sungguhan belum dicoba.
