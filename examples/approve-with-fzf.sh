#!/bin/bash
# Example script showing how to use fzf to select and approve pairing requests
#
# Usage: ./approve-with-fzf.sh
#
# This script lists all pending pairing requests and lets you select one with fzf
# to approve it.

set -e

# Check if fzf is installed
if ! command -v fzf &> /dev/null; then
    echo "Error: fzf is not installed. Please install it first."
    echo "  On Ubuntu/Debian: sudo apt install fzf"
    echo "  On Arch: sudo pacman -S fzf"
    echo "  On macOS: brew install fzf"
    exit 1
fi

# Check if handcontrol binary exists
if [ ! -f "./target/debug/handcontrol" ]; then
    echo "Error: handcontrol binary not found. Please run 'cargo build' first."
    exit 1
fi

# Get list of pending pairing requests
# Format: request_id device_name device_model pin expires_in_seconds
SELECTED=$(./target/debug/handcontrol list-pending 2>/dev/null | \
    fzf --delimiter='\t' \
        --with-nth=2,3,4,5 \
        --header='Select a pairing request to approve' \
        --preview='echo "Request ID: {1}\nDevice: {2}\nModel: {3}\nPIN: {4}\nExpires in: {5}"' \
        --preview-window=up:5)

if [ -z "$SELECTED" ]; then
    echo "No request selected."
    exit 0
fi

# Extract request_id (first column)
REQUEST_ID=$(echo "$SELECTED" | cut -f1)

echo "Approving request: $REQUEST_ID"
./target/debug/handcontrol approve "$REQUEST_ID"
