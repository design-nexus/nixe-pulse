#!/usr/bin/env bash
# Install Pulse using the suite installer. Options pass through
# (see ../../install.sh).
here=$(cd "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
exec "$here/../../install.sh" pulse "$@"
