#!/bin/sh
# The same check from Git Bash or other shells (CrabLogic runs on Windows): see check.ps1.
#   check.sh <folder or file>   check.sh (hook, JSON on stdin)   check.sh -Stop
exec powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(dirname "$0")/check.ps1" "$@"
