#!/bin/bash

# Configure gh auth from GH_TOKEN if available
if [ -n "$GH_TOKEN" ]; then
    gh auth setup-git 2>/dev/null || true
fi

# Configure git identity (run from home dir to avoid worktree .git issues)
cd "$HOME" 2>/dev/null || cd /tmp
git config --global user.name "Reckoner"
git config --global user.email "reckoner@local"
git config --global init.defaultBranch main

# Return to workspace
cd /workspace 2>/dev/null || true

# Execute the provided command
exec "$@"
