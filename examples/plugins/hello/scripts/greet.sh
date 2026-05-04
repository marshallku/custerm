#!/bin/bash
# Reads JSON params from stdin, returns a greeting

PARAMS=$(cat)
NAME=$(echo "$PARAMS" | python3 -c "import sys, json; print(json.load(sys.stdin).get('name', 'world'))" 2>/dev/null || echo "world")
TIMESTAMP=$(date -Iseconds)

cat <<EOF
{
    "message": "Hello, ${NAME}!",
    "plugin_dir": "${NESTTY_PLUGIN_DIR}",
    "socket": "${NESTTY_SOCKET}",
    "timestamp": "${TIMESTAMP}"
}
EOF
