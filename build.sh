#!/bin/bash

# Polymarket Arbitrage Bot - Build Script
set -e

echo "================================"
echo "Polymarket Arb Bot - Build Script"
echo "================================"
echo ""

# Check if Rust is installed
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust is not installed!"
    echo "Please install Rust from: https://rustup.rs"
    echo ""
    echo "Quick install:"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

echo "✅ Rust detected: $(rustc --version)"
echo ""

# Check if .env exists
if [ ! -f ".env" ]; then
    echo "⚠️  .env file not found!"
    echo "Creating from .env.example..."
    cp .env.example .env
    echo "✅ Created .env file"
    echo ""
    echo "⚠️  IMPORTANT: Edit .env and add your PRIVATE_KEY!"
    echo "   nano .env"
    echo ""
    read -p "Press Enter after you've edited .env file..."
fi

# Check if private key is set
if grep -q "your_private_key_here" .env; then
    echo "⚠️  WARNING: PRIVATE_KEY still set to placeholder!"
    echo "   Please edit .env and set your actual private key"
    echo ""
    read -p "Continue anyway? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# Build the project
echo "🔨 Building in release mode..."
echo "   (This may take 2-5 minutes on first build)"
echo ""

cargo build --release

if [ $? -eq 0 ]; then
    echo ""
    echo "✅ Build successful!"
    echo ""
    echo "Binary location: target/release/polymarket-arb-bot"
    echo ""
    echo "Next steps:"
    echo "  1. Review .env configuration"
    echo "  2. Test in dry-run mode: ./target/release/polymarket-arb-bot"
    echo "  3. Set DRY_RUN=false in .env when ready for live trading"
    echo ""
else
    echo ""
    echo "❌ Build failed!"
    echo "Check error messages above"
    exit 1
fi