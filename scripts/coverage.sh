#!/usr/bin/env bash
# ==============================================================================
# Script de Calcul de Couverture de Code AI-DFDL (via cargo-llvm-cov)
#
# Usage:
#   ./scripts/coverage.sh unit         # Couverture tests unitaires uniquement
#   ./scripts/coverage.sh conformance  # Couverture tests de conformance TDML uniquement
#   ./scripts/coverage.sh all          # Couverture combinée (unitaires + conformance)
#
# Options:
#   --html                             # Génère un rapport HTML interactif
#   --lcov                             # Génère un fichier lcov.info pour CI / Sonar
#   --open                             # Ouvre automatiquement le rapport HTML
# ==============================================================================

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

MODE="unit"
FORMAT_FLAGS=()
OUTPUT_DIR=""
OPEN_BROWSER=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        unit|conformance|all|combined)
            MODE="$1"
            shift
            ;;
        --html)
            FORMAT_FLAGS+=("--html")
            shift
            ;;
        --open)
            FORMAT_FLAGS+=("--html" "--open")
            OPEN_BROWSER=1
            shift
            ;;
        --lcov)
            FORMAT_FLAGS+=("--lcov")
            shift
            ;;
        --json)
            FORMAT_FLAGS+=("--json")
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [unit|conformance|all] [--html] [--lcov] [--json] [--open]"
            exit 0
            ;;
        *)
            echo "Option inconnue: $1" >&2
            echo "Usage: $0 [unit|conformance|all] [--html] [--lcov] [--json] [--open]" >&2
            exit 1
            ;;
    esac
done

if [[ ${#FORMAT_FLAGS[@]} -eq 0 ]]; then
    FORMAT_FLAGS=("--summary-only")
fi

# Vérification de l'installation de cargo-llvm-cov
if ! command -v cargo-llvm-cov &>/dev/null; then
    echo "cargo-llvm-cov n'est pas installé. Installation..."
    cargo install cargo-llvm-cov --locked
fi

# Exclusion des harnesses de test pour mesurer exclusivement le moteur de production
EXCLUDES=("--exclude-from-report" "dfdl-tests" "--exclude-from-report" "dfdl-fuzz")

echo "=============================================================================="
echo "AI-DFDL — Calcul de Couverture de Code"
echo "Mode sélectionné : ${MODE}"
echo "=============================================================================="

case "${MODE}" in
    unit)
        OUTPUT_DIR="target/llvm-cov/unit"
        mkdir -p "${OUTPUT_DIR}"
        echo "Exécution des tests unitaires du workspace..."
        cargo llvm-cov --workspace "${EXCLUDES[@]}" "${FORMAT_FLAGS[@]}" --output-dir "${OUTPUT_DIR}"
        ;;
    conformance)
        OUTPUT_DIR="target/llvm-cov/conformance"
        mkdir -p "${OUTPUT_DIR}"
        echo "Exécution de la suite officielle de conformance Apache Daffodil TDML (4 336 tests)..."
        cargo llvm-cov test test_apache_daffodil_official_tdml_suite \
            --release \
            "${EXCLUDES[@]}" \
            "${FORMAT_FLAGS[@]}" \
            --output-dir "${OUTPUT_DIR}" \
            -- --ignored
        ;;
    all|combined)
        OUTPUT_DIR="target/llvm-cov/combined"
        mkdir -p "${OUTPUT_DIR}"
        echo "Exécution combinée des tests unitaires et de la conformance TDML..."
        cargo llvm-cov test \
            --release \
            "${EXCLUDES[@]}" \
            "${FORMAT_FLAGS[@]}" \
            --output-dir "${OUTPUT_DIR}" \
            -- --include-ignored
        ;;
esac

echo "=============================================================================="
echo "Couverture terminée avec succès !"
if [[ " ${FORMAT_FLAGS[*]} " =~ " --html " ]]; then
    echo "Rapport HTML disponible dans : ${OUTPUT_DIR}/html/index.html"
fi
echo "=============================================================================="
