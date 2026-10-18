import argparse
import json
import secrets
from pathlib import Path


def make_secret() -> str:
    return secrets.token_hex(32)


def main():
    parser = argparse.ArgumentParser(description="Generate AI solver client configs and server CLIENT_SECRETS line.")
    parser.add_argument(
        "--server-url",
        required=True,
        help="Public solve endpoint, e.g. https://your-domain.example/solve",
    )
    parser.add_argument(
        "--clients",
        nargs="+",
        required=True,
        help="Client IDs to generate, e.g. emre-pc friend1-laptop",
    )
    parser.add_argument(
        "--outdir",
        default="generated_clients",
        help="Directory to write generated client config folders into",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=45,
        help="Client timeout_seconds value",
    )
    parser.add_argument(
        "--mode",
        default="solution",
        help="Default client mode",
    )
    parser.add_argument(
        "--gemini-api-key",
        default="",
        help="Gemini API key for the client-side fallback (written into each client config)",
    )

    args = parser.parse_args()

    outdir = Path(args.outdir)
    outdir.mkdir(parents=True, exist_ok=True)

    client_secrets_parts = []

    for client_id in args.clients:
        secret = make_secret()
        client_secrets_parts.append(f"{client_id}:{secret}")

        client_dir = outdir / client_id
        client_dir.mkdir(parents=True, exist_ok=True)

        config = {
            "server_url": args.server_url,
            "client_id": client_id,
            "client_secret": secret,
            "timeout_seconds": args.timeout,
            "mode": args.mode,
        }
        if args.gemini_api_key:
            config["gemini_api_key"] = args.gemini_api_key

        config_path = client_dir / "client_config.json"
        with config_path.open("w", encoding="utf-8") as f:
            json.dump(config, f, indent=2)

    client_secrets_line = "CLIENT_SECRETS=" + ",".join(client_secrets_parts)

    print("\nServer .env line:\n")
    print(client_secrets_line)

    print("\nGenerated client config files:\n")
    for client_id in args.clients:
        print(outdir / client_id / "client_config.json")


if __name__ == "__main__":
    main()