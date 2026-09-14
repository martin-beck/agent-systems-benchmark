#!/bin/sh
# Copyright (C) Huawei Technologies Co., Ltd. 2026. All rights reserved.
# SPDX-License-Identifier: MIT
cat >/dev/null
printf "ASB_TUI_RENDERED\n" >/dev/tty
IFS= read -r key </dev/tty
[ "$key" = q ] || exit 3
printf "%s\n" '{"schema_version":1,"classification":"verified_extension","ok":true,"code":"frontend_exited"}'
