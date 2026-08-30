#!/bin/bash
# Plain http is a secure origin on localhost only; a phone on the LAN needs TLS.
set -euo pipefail
cd "$(dirname "$0")/../platforms/web"
exec python3 -m http.server "${PORT:-8080}"
