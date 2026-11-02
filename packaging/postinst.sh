#!/bin/sh
# Runs after the package is installed or upgraded: lets the system know about
# the virtual camera rule. Nothing here may fail the installation.
udevadm control --reload-rules >/dev/null 2>&1 || true
udevadm trigger --subsystem-match=video4linux >/dev/null 2>&1 || true
exit 0
