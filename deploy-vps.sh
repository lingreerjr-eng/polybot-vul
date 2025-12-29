#!/bin/bash

# VPS Deployment Script for Polymarket Arb Bot
set -e

echo "================================"
echo "Polymarket Arb Bot - VPS Setup"
echo "================================"
echo ""

# Check if running as root
if [ "$EUID" -eq 0 ]; then 
    echo "❌ Please don't run as root!"
    echo "Run as regular user: ./deploy-vps.sh"
    exit 1
fi

USERNAME=$(whoami)
WORK_DIR=$(pwd)
SERVICE_NAME="polymarket-bot"

echo "User: $USERNAME"
echo "Working directory: $WORK_DIR"
echo ""

# Install system dependencies
echo "📦 Installing system dependencies..."
sudo apt update
sudo apt install -y build-essential pkg-config libssl-dev git curl

# Install Rust if not already installed
if ! command -v cargo &> /dev/null; then
    echo "📦 Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source $HOME/.cargo/env
else
    echo "✅ Rust already installed: $(rustc --version)"
fi

# Check .env file
if [ ! -f ".env" ]; then
    echo "⚠️  Creating .env from template..."
    cp .env.example .env
    echo "⚠️  IMPORTANT: Edit .env and add your PRIVATE_KEY!"
    echo "   nano .env"
    read -p "Press Enter after editing .env..."
fi

# Build the project
echo "🔨 Building project..."
cargo build --release

# Create systemd service file
echo "⚙️  Creating systemd service..."

SERVICE_FILE="/tmp/${SERVICE_NAME}.service"

cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=Polymarket Arbitrage Bot
After=network.target

[Service]
Type=simple
User=$USERNAME
WorkingDirectory=$WORK_DIR
Environment="RUST_LOG=info"
ExecStart=$WORK_DIR/target/release/polymarket-arb-bot
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

sudo mv "$SERVICE_FILE" "/etc/systemd/system/${SERVICE_NAME}.service"
sudo chmod 644 "/etc/systemd/system/${SERVICE_NAME}.service"

# Reload systemd
echo "🔄 Reloading systemd..."
sudo systemctl daemon-reload

# Enable service
echo "✅ Enabling service to start on boot..."
sudo systemctl enable "$SERVICE_NAME"

echo ""
echo "✅ Deployment complete!"
echo ""
echo "Service commands:"
echo "  Start:   sudo systemctl start $SERVICE_NAME"
echo "  Stop:    sudo systemctl stop $SERVICE_NAME"
echo "  Restart: sudo systemctl restart $SERVICE_NAME"
echo "  Status:  sudo systemctl status $SERVICE_NAME"
echo "  Logs:    sudo journalctl -u $SERVICE_NAME -f"
echo ""
echo "⚠️  IMPORTANT: Review your .env file before starting!"
echo ""

read -p "Start the bot now? (y/N) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    sudo systemctl start "$SERVICE_NAME"
    echo ""
    echo "🚀 Bot started!"
    echo ""
    echo "View logs with: sudo journalctl -u $SERVICE_NAME -f"
fi