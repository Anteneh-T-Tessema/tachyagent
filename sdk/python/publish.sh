#!/bin/bash
# Publish the Tachy Python SDK to PyPI.
# Prerequisites: pip install build twine
# Set TWINE_USERNAME and TWINE_PASSWORD (or use __token__ + PyPI API token)
set -e

echo "Building..."
python3 -m build

echo "Uploading to PyPI..."
python3 -m twine upload dist/*

echo "Done. Install with: pip install tachy-agent"
