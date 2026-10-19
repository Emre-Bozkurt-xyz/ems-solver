# ems-solver

A small tray app: copy a question or take a screenshot, press a hotkey, and the answer is
pasted where your cursor is. Answers come from Claude through an ems-solver server, or
directly from Gemini with your own free key, or both (Gemini then takes over when the
server is unreachable).

## Install

Download the newest release from the [Releases page](../../releases/latest).

- **Windows:** run `ems-solver_x.y.z_x64-setup.exe`. Windows may warn about an unknown
  publisher; choose **More info → Run anyway**. No admin rights are needed.
- **Ubuntu / Debian:** `sudo apt install ./ems-solver_x.y.z_amd64.deb`
- **Other Linux:** download the `.AppImage`, `chmod +x` it and run it.

ems-solver updates itself: when a new version is out, it tells you, and you install it
from the tray menu or the settings window.

## Set up

The settings window opens on first start (later: tray icon → **Settings and status…**).
Fill in one or both of:

- **Your server (Claude):** server URL, client ID and client secret. Ask whoever runs
  the server.
- **Gemini:** an API key from [aistudio.google.com](https://aistudio.google.com/apikey)
  (free). Use it alone, or as a fallback for when the server is down.

Use the **Test** buttons, then **Save**. Secrets go into your system's credential store.

## Use

| Action | Default hotkey | What it does |
|---|---|---|
| Add | `Ctrl+Alt+Shift+A` | Adds the clipboard (text or a screenshot) to the buffer |
| Send | `Ctrl+Alt+Shift+S` | Sends the buffer, or just the clipboard if the buffer is empty |
| Clear | `Ctrl+Alt+Shift+X` | Empties the buffer |

Typical uses:

- **One question:** copy it (or screenshot it with `Win+Shift+S`), press **Send**.
- **Several pieces:** copy/screenshot each piece and press **Add** after each one, then
  **Send**. When there are several questions, the last one is answered and the rest are
  used as context.

The tray icon shows what's happening: blue dot = something in the buffer, amber = waiting
for the answer, red = the last request failed (hover for details), grey = not set up.

The tray menu also switches the **mode** (full solution, a hint, or check my work) and
where answers go: **pasted at the cursor**, or **copied** for you to paste yourself.

### Ubuntu (and other Wayland desktops)

Wayland doesn't let apps register global hotkeys, so add them yourself:
**Settings → Keyboard → View and Customize Shortcuts → Custom Shortcuts → +**, once for
each of these commands:

```
ems-solver add
ems-solver send
ems-solver clear
```

GNOME on Wayland also doesn't allow apps to type into other windows, so answers are put on
the clipboard and you paste them with `Ctrl+V`. To get automatic pasting, log in with
**Ubuntu on Xorg** (gear icon on the login screen), or install `ydotool` and run its
daemon. **Settings → This system** shows what works on your machine and how to fix the rest.

### From a terminal

```
ems-solver ask "What is 17 * 23?"            # prints the answer
ems-solver ask --image shot.png "Solve this"  # with a screenshot
ems-solver capabilities                       # what this system supports
ems-solver test                               # check the server and Gemini settings
ems-solver help                               # everything else
```

## Troubleshooting

- **Nothing happens on a hotkey:** another app may own that combination. Settings shows
  hotkeys that couldn't be registered; record different ones.
- **"No answer":** hover over the tray icon or check the log (Settings → Files →
  **Open folder**, file `ems-solver.log`).
- **Pasted text looks wrong in a terminal:** terminals paste with `Ctrl+Shift+V`; switch
  the output to **Copy to the clipboard** while you work there.

---

## Development

```
server/   FastAPI service that calls Claude (Python 3.12)
client/   Tauri v2 desktop app (Rust + a plain HTML settings page)
docs/     api.md: the server API and request signing
legacy/   the original Espanso-based version
```

**Server:** `cd server`, `pip install -r requirements-dev.txt`, `pytest`. Run locally with
`uvicorn ems_solver.main:app --port 8011` and a `.env` based on `.env.example`.
`scripts/smoke.py` sends signed test requests.

**Client:** `cd client && npm ci && npx tauri dev`, or `cargo test` in `client/src-tauri`.
Linux needs `libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev`.

**Prompt:** `server/ems_solver/system_prompt.md` is used by the server and, for the Gemini
path, compiled into the client.

### Deploying the server

Pushes to `main` that touch `server/` run the tests and then deploy from a GitHub-hosted
runner: SSH through Cloudflare Access to `deploy@` the host, sync `server/` into
`/opt/apps/ems-solver/app`, install requirements and restart the `ems-solver` service. The host
is prepared once with `server/deploy/setup-server.sh` (usage is at the top of the file). The service listens on `127.0.0.1:8011`; frp and the VPS publish it as
`solver.ems-place.com`. Settings live in `/opt/apps/ems-solver/.env` on the host.

Repository secrets: `CF_ACCESS_CLIENT_ID`, `CF_ACCESS_CLIENT_SECRET`, `DEPLOY_SSH_KEY`,
`DEPLOY_KNOWN_HOSTS`, `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
Repository variable: `DEPLOY_HOST`.

Adding a client: append `id:secret` to `CLIENT_SECRETS` in the host's `.env`
(generate a secret with `python -c "import secrets; print(secrets.token_hex(32))"`) and
restart the service.

### Releasing the app

```
node client/scripts/version.mjs set 0.2.0
git commit -am "Release 0.2.0" && git tag v0.2.0 && git push && git push --tags
```

The Release workflow builds the Windows installer and the Linux `.deb`/`.AppImage` into a
**draft** release. Check it, then publish it; published releases are what installed apps
update to. Updates are signed: keep a backup of the updater signing key, because without
it you can't ship updates to existing installs.
