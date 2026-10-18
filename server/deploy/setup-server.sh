#!/usr/bin/env bash
# One-time setup of a host for ems-solver deploys from GitHub Actions. Run as root:
#
#   sudo bash setup-server.sh --deploy-key deploy_key.pub [--env-from OLD.env] [--disable-password-login]
#
# Creates the `deploy` user (key-only, no Docker access), /opt/ems-solver, the systemd
# unit, and a sudo rule that only allows restarting the service and reading its logs.
# Safe to run again.
set -euo pipefail

usage() { sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 1; }

DEPLOY_KEY="" ENV_FROM="" DISABLE_PASSWORDS=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --deploy-key) DEPLOY_KEY=${2:?}; shift 2 ;;
    --env-from) ENV_FROM=${2:?}; shift 2 ;;
    --disable-password-login) DISABLE_PASSWORDS=1; shift ;;
    *) usage ;;
  esac
done
[[ -n $DEPLOY_KEY ]] || usage
[[ $EUID -eq 0 ]] || { echo "Run this with sudo." >&2; exit 1; }
[[ -s $DEPLOY_KEY ]] || { echo "Deploy key $DEPLOY_KEY is missing or empty." >&2; exit 1; }

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=/opt/ems-solver
SERVICE=ems-solver

step() { printf '\n== %s\n' "$*"; }

step "Packages"
missing=()
dpkg -s python3-venv >/dev/null 2>&1 || missing+=(python3-venv)
command -v rsync >/dev/null || missing+=(rsync)
command -v curl >/dev/null || missing+=(curl)
if ((${#missing[@]})); then
  apt-get update -qq
  apt-get install -y -qq "${missing[@]}"
fi
echo "ok"

step "deploy user"
id deploy >/dev/null 2>&1 || useradd --create-home --shell /bin/bash deploy
# '*' means "no password" without marking the account locked, which would block key logins.
usermod -p '*' deploy
install -d -m 700 -o deploy -g deploy /home/deploy/.ssh
# `restrict` turns off port/agent forwarding and terminals; running commands (rsync, the deploy script) still works.
printf 'restrict %s\n' "$(head -n1 "$DEPLOY_KEY")" > /home/deploy/.ssh/authorized_keys
chown deploy:deploy /home/deploy/.ssh/authorized_keys
chmod 600 /home/deploy/.ssh/authorized_keys
echo "ok"

step "$ROOT"
install -d -m 750 -o deploy -g deploy "$ROOT" "$ROOT/app"
if [[ ! -f $ROOT/.env ]]; then
  if [[ -n $ENV_FROM ]]; then
    sed 's/\r$//' "$ENV_FROM" > "$ROOT/.env"
    sed -i -E 's/^ANTHROPIC_MODEL=claude-sonnet-4-6$/ANTHROPIC_MODEL=claude-sonnet-5-5/' "$ROOT/.env"
    echo "copied settings from $ENV_FROM"
  else
    cat > "$ROOT/.env" <<'EOF'
ANTHROPIC_API_KEY=
ANTHROPIC_MODEL=claude-sonnet-5-5
# client_id:secret pairs, comma-separated
CLIENT_SECRETS=
EOF
    echo "created an empty $ROOT/.env; fill it in before the first deploy"
  fi
  chown deploy:deploy "$ROOT/.env"
  chmod 600 "$ROOT/.env"
else
  echo "$ROOT/.env already exists; left as is"
fi

step "systemd unit"
install -m 644 "$HERE/$SERVICE.service" "/etc/systemd/system/$SERVICE.service"
systemctl daemon-reload
# Started by the first deploy, once there is code in $ROOT/app.
systemctl enable "$SERVICE" >/dev/null
echo "ok"

step "sudo rule"
rule=/etc/sudoers.d/$SERVICE-deploy
cat > "$rule.tmp" <<EOF
deploy ALL=(root) NOPASSWD: /usr/bin/systemctl restart $SERVICE, /usr/bin/journalctl -u $SERVICE -n 50 --no-pager
EOF
chmod 440 "$rule.tmp"
visudo -cf "$rule.tmp" >/dev/null
mv "$rule.tmp" "$rule"
echo "ok"

if ((DISABLE_PASSWORDS)); then
  step "Password login"
  # Make sure whoever ran this can still get in with a key afterwards.
  invoker_keys=$(getent passwd "${SUDO_USER:-root}" | cut -d: -f6)/.ssh/authorized_keys
  if [[ ! -s $invoker_keys ]]; then
    echo "Skipped: ${SUDO_USER:-root} has no SSH keys in $invoker_keys, so this could lock you out." >&2
  else
    # sshd uses the first value it reads, and files are read in name order, so 00- wins.
    echo "PasswordAuthentication no" > /etc/ssh/sshd_config.d/00-no-password-login.conf
    sshd -t
    systemctl reload ssh 2>/dev/null || systemctl restart ssh
    echo "password login disabled (existing sessions stay open)"
  fi
fi

step "Done"
echo "Deploys can now log in as deploy@$(hostname) and restart $SERVICE."
