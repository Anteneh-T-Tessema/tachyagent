#!/bin/bash
# Publish the Tachy VS Code extension to the marketplace.
# Prerequisites: npm install -g @vscode/vsce
# Set VSCE_PAT to your Personal Access Token
set -e

echo "Installing dependencies..."
npm install

echo "Compiling TypeScript..."
npm run compile

echo "Packaging..."
vsce package

echo "Publishing..."
vsce publish

echo "Done."
