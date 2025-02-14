#!/bin/bash

if [[ " $* " =~ " --fix " ]]; then
    prettier --write "**/*.md"
    prettier --write "**/*.{yaml,yml}"
    cargo fmt --all
else
    prettier --check "**/*.md"
    prettier --check "**/*.{yaml,yml}"
    cargo fmt --all --check
fi
