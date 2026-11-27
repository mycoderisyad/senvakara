# Bundled module catalog

Every folder `modules/<category>/<id>/` with a `module.toml` is one module; `build.rs`
embeds all of its files and `Catalog::bundled()` checks them (plan WF §2, ADR-029/030/031,
ADR-033). The format, the scripts and the compose files are written in-house: steps follow
the vendor's own documentation (named in `docs_url`), nothing is copied or fetched from
other platforms, and no third-party logo or branding ships with a module.

## Files

| File | Runs as | Contract |
|---|---|---|
| `module.toml` | — | id `category.name`, name, summary, version (bump on every script change), `os = ["debian>=12", "ubuntu>=22.04"]`, `needs`, `timeout_secs`, `params`, `docs_url` |
| `check.sh` | root when `needs` has `root` | Read-only. Exit 0 = up to date, 10 = needs apply; any other status fails the step. Prints one short sentence saying why |
| `apply.sh` | same | Idempotent: running it twice changes nothing the second time. Must leave `check.sh` at exit 0 |
| `health.sh` | **the signed-in user, never sudo** | Read-only, on demand only. Ends with exactly one line `SV_HEALTH <running\|degraded\|stopped\|unknown> <detail>` and exits 0. May wait up to `$SV_HEALTH_WAIT` seconds for the service to come up (0 on demand, up to 90 right after an apply); otherwise answers within a few seconds. Uses bash, coreutils and `systemctl` only (no curl, no docker CLI: the user may not be in the docker group). A TCP probe: `timeout 2 bash -c ': >/dev/tcp/127.0.0.1/PORT'` |
| `remove.sh` | same as apply | Started only by the user's explicit, confirmed Remove. Undoes what `apply.sh` added (service, packages, repository, compose project) but **never deletes data**: it prints where data was left |
| `rollback.sh` | root | Guarded modules only (ADR-031) |
| other files (`compose.yml`, ...) | — | Uploaded with the scripts into the private run folder; scripts find them in `$SV_MODULE_DIR` (read-only copy, removed after the run) |

## Settings

- Every parameter reaches the scripts as `SV_<NAME>`. Ordinary values are `export` lines at
  the top of the script file. `secret` values (kept in Credential Manager, ADR-033) are never
  written to the script file or argv: the launcher receives them on stdin and exports them
  before the script starts. Scripts must not echo a secret, must not use `set -x`, and write
  secrets only into files of mode 0600. `health.sh` never receives secrets.
- Health states: `running` = it serves; `degraded` = up but not fully (unit active while its
  port is silent, still starting, one of two ports answers); `stopped` = not running, failed,
  or not installed (an app whose port stays silent counts as stopped); `unknown` = cannot tell
  without root. Right after an apply, `stopped` fails the step.
- Engine names (`SV_SESSION_*`, `SV_GUARD_*`, `SV_MODULE_DIR`, `SV_HEALTH_WAIT`) cannot be
  parameter names.

## Conventions

- apt: `export DEBIAN_FRONTEND=noninteractive LC_ALL=C`, `apt-get -o DPkg::Lock::Timeout=300`.
  Vendor repositories use a keyring in `/etc/apt/keyrings/` and a deb822 file
  `/etc/apt/sources.list.d/<vendor>.sources` with `Signed-By:`, written only when its text
  differs (so a second apply changes nothing). Pin the versions the vendor lets you pin
  (a major-version choice, an image tag), never `latest`.
- Docker apps (category `app`): Docker Engine with the compose plugin must already be there
  (`runtime.docker`; `check.sh` exits 2 with that sentence otherwise). The app lives in
  `/opt/senvakara/apps/<id>/` (0755): `compose.yml` copied from `$SV_MODULE_DIR` (0644) and
  `.env` (0600, root) rendered from the settings; the compose project is named
  `sv-<id>` by the file's `name:`. Values reach the compose file only through `${VAR}`.
  Published ports bind to a `bind` setting that defaults to `127.0.0.1`: Docker's published
  ports bypass ufw, so exposing an app is an explicit choice (or put Caddy/Nginx in front).
- Scripts run with `set -euo pipefail`; messages are plain sentences, no colours.
