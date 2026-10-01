#!/usr/bin/env bash
# Render every PlantUML diagram in this directory.
#   ./render.sh          -> SVG
#   ./render.sh pdf      -> PDF (for camera-ready figures)
#   ./render.sh png      -> PNG
set -euo pipefail

FORMAT="${1:-svg}"
cd "$(dirname "$0")"

if command -v plantuml >/dev/null 2>&1; then
    plantuml "-t${FORMAT}" ./*.puml
elif [[ -n "${PLANTUML_JAR:-}" && -f "${PLANTUML_JAR}" ]]; then
    java -jar "${PLANTUML_JAR}" "-t${FORMAT}" ./*.puml
else
    echo "Need either 'plantuml' on PATH (brew install plantuml) or PLANTUML_JAR=/path/to/plantuml.jar" >&2
    exit 1
fi

echo "Rendered $(ls -1 ./*.puml | wc -l | tr -d ' ') diagrams to ${FORMAT}."
