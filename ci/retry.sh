#!/usr/bin/env bash
# PLAN-RELEASE-1 S2 — retry wrapper for network-dependent CI steps.
#
#   ci/retry.sh [-n attempts] -- <cmd> [args...]
#
# Re-runs a flaky network step (rustup/component downloads, cargo-fuzz
# install) with exponential backoff (5s, 20s, 80s, …). Exits 0 on the first
# success, or with the last attempt's failure status. Used ONLY for network
# steps — build/test steps fail closed on the first attempt.
set -euo pipefail

attempts=3
while getopts "n:" opt; do
    case "$opt" in
        n) attempts="$OPTARG" ;;
        *) echo "usage: $0 [-n attempts] -- <cmd> [args...]" >&2; exit 2 ;;
    esac
done
shift $((OPTIND - 1))

if [ "$#" -lt 1 ]; then
    echo "usage: $0 [-n attempts] -- <cmd> [args...]" >&2
    exit 2
fi

backoff=5
last=0
for i in $(seq 1 "$attempts"); do
    "$@"
    status=$?
    if [ "$status" -eq 0 ]; then
        exit 0
    fi
    last=$status
    if [ "$i" -lt "$attempts" ]; then
        echo "retry: attempt $i/$attempts failed ($status) — retrying in ${backoff}s" >&2
        sleep "$backoff"
        backoff=$((backoff * 4))
    fi
done
exit "$last"