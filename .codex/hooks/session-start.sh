#!/bin/bash
# Linuxリモート準備の手順は共通入口だけに置く。
set -euo pipefail
exec bash "$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/tools/session-start.sh"
