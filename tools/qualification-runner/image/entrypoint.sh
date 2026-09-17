#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
set -eu
case "${1:-}" in
  capability-probe) exec /opt/asb/bin/capability-probe ;;
  exec) shift; [ "$#" -gt 0 ] || { echo "missing executable" >&2; exit 64; }; exec "$@" ;;
  *) echo "usage: capability-probe | exec PROGRAM [ARGS...]" >&2; exit 64 ;;
esac
