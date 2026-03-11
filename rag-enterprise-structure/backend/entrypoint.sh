#!/bin/bash
set -e

# Install/update Python dependencies to handle volume-mounted code changes
echo "Checking Python dependencies..."
pip install --no-cache-dir -q -r /app/requirements.txt 2>/dev/null || \
    echo "Warning: Could not install some dependencies from requirements.txt"

# Start the application
exec python3 -m uvicorn app:app --host 0.0.0.0 --port 8000
