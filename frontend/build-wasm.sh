#!/bin/bash
set -e

echo "building wasm module..."

cd ../wasm

# check if wasm-pack is installed
if ! command -v wasm-pack &> /dev/null; then
    echo "error: wasm-pack is not installed"
    echo "install it with: cargo install wasm-pack"
    exit 1
fi

# build the wasm module
wasm-pack build --target web --out-dir pkg

echo "wasm module built successfully!"
echo "you can now run 'npm run dev' in the frontend directory"
