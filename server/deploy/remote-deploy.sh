#!/usr/bin/env bash
# Runs on the server as `deploy`, after the workflow has synced server/ into /opt/ems-solver/app.
set -euo pipefail

ROOT=/opt/ems-solver
cd "$ROOT"

if [[ ! -x venv/bin/python ]]; then
  python3 -m venv venv
fi
venv/bin/pip install --quiet --disable-pip-version-check -r app/requirements.txt

sudo -n /usr/bin/systemctl restart ems-solver

for _ in $(seq 1 30); do
  if curl -fsS http://172.17.0.1:8011/healthz >/dev/null 2>&1; then
    echo "ems-solver is up"
    exit 0
  fi
  sleep 1
done

echo "ems-solver didn't come up within 30 s. Recent logs:" >&2
sudo -n /usr/bin/journalctl -u ems-solver -n 50 --no-pager >&2 || true
exit 1
