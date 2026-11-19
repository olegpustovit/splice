#!/bin/sh
# Outcome evals (PLAN.md T3.6). See tools/eval.py.
exec python3 "$(dirname "$0")/eval.py" "$@"
