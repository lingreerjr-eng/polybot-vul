# Polymarket Arbitrage Bot (Rust)

High-performance arbitrage bot for Polymarket 15-minute binary options markets, converted from Python to Rust using the official `polymarket-client-sdk` (rs-clob-client).

## Features

- **Real-time WebSocket monitoring** of market prices
- **Dynamic bankroll sizing** based on available balance
- **Mismatch handling** (refill or unwind partial fills)
- **Automatic market rollover** every 15 minutes
- **Configurable thresholds** for risk management
- **Dry-run mode** for testing without real trades

## Prerequisites

### System Requirements
- **Rust 1.88+** (install from https://rustup.rs)
- **Linux, macOS, or Windows** with WSL
- **Stable internet connection**
- **Polymarket account** with funded wallet

### Before Running
1. **Set token allowances** on Polymarket (required for EOA wallets)
   - Go to Polymarket settings
   - Approve USDC and conditional tokens
   - See: https://docs.polymarket.com/developers/

2. **Get your private key**
   - MetaMask: Account Details > Export Private Key
   - Other wallets: Check wallet documentation
   - ⚠️ **NEVER share your private key**

## Quick Start (Laptop Testing)

### 1. Install Rust

```bash
# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Verify installation
rustc --version
cargo --version
```

### 2. Download & Extract

```bash
# Extract the zip file
unzip polymarket-arb-bot.zip
cd polymarket-arb-bot
```

### 3. Configure Environment

```bash
# Copy environment template
cp .env.example .env

# Edit configuration
nano .env  # or use your preferred editor
```

**Required changes in `.env`:**
```bash
PRIVATE_KEY=your_actual_private_key_here_without_0x_prefix
DRY_RUN=true  # Keep as true for testing!
```

**Optional adjustments:**
- `SIZE=5` - Max shares per leg (fallback if balance fetch fails)
- `SUM_MAX=0.97` - Maximum sum of both prices (arb threshold)
- `BANKROLL_FRACTION=0.25` - Use 25% of available balance per trade
- `MIN_TRADE_NOTIONAL_USD=2.0` - Minimum total trade size

### 4. Build the Project

```bash
# Build in release mode (optimized)
cargo build --release

# This will take 2-5 minutes on first build
# Binary will be at: target/release/polymarket-arb-bot
```

### 5. Test Run (Dry Mode)

```bash
# Run with dry-run enabled
RUST_LOG=info cargo run --release

# Or run the binary directly
./target/release/polymarket-arb-bot
```

**What to expect:**
- Bot connects to WebSocket
- Resolves current 15m markets for BTC, ETH, SOL, XRP
- Monitors order book updates
- Logs arbitrage opportunities (without executing trades)

### 6. Go Live (Disable Dry Run)

Once you've verified everything works:

```bash
# Edit .env and change:
DRY_RUN=false

# Run live
cargo run --release
```

## VPS Deployment

### Recommended VPS Specs
- **RAM:** 2GB minimum (4GB recommended)
- **CPU:** 2 cores
- **Storage:** 10GB
- **OS:** Ubuntu 22.04 LTS or Debian 12
- **Providers:** DigitalOcean, Linode, AWS EC2, Vultr

### Setup Steps

#### 1. Connect to VPS

```bash
ssh user@your-vps-ip
```

#### 2. Install Dependencies

```bash
# Update system
sudo apt update && sudo apt upgrade -y

# Install build essentials
sudo apt install -y build-essential pkg-config libssl-dev git curl

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

#### 3. Upload Bot Files

**Option A: Upload zip file**
```bash
# From your laptop
scp polymarket-arb-bot.zip user@your-vps-ip:~/

# On VPS
cd ~
unzip polymarket-arb-bot.zip
cd polymarket-arb-bot
```

**Option B: Git (if using private repository)**
```bash
# On VPS
git clone https://github.com/yourusername/polymarket-arb-bot.git
cd polymarket-arb-bot
```

#### 4. Configure Environment

```bash
# Create .env file
cp .env.example .env
nano .env

# Set your private key and other parameters
# Start with DRY_RUN=true for testing
```

#### 5. Build on VPS

```bash
# Build optimized binary
cargo build --release

# Binary location: target/release/polymarket-arb-bot
```

#### 6. Test Run

```bash
# Quick test
./target/release/polymarket-arb-bot

# Press Ctrl+C to stop after confirming it works
```

#### 7. Run as Background Service (systemd)

Create a systemd service file:

```bash
sudo nano /etc/systemd/system/polymarket-bot.service
```

Add this configuration:

```ini
[Unit]
Description=Polymarket Arbitrage Bot
After=network.target

[Service]
Type=simple
User=your-username
WorkingDirectory=/home/your-username/polymarket-arb-bot
Environment="RUST_LOG=info"
ExecStart=/home/your-username/polymarket-arb-bot/target/release/polymarket-arb-bot
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

**Important:** Replace `your-username` with your actual Linux username!

Enable and start the service:

```bash
# Reload systemd
sudo systemctl daemon-reload

# Enable bot to start on boot
sudo systemctl enable polymarket-bot

# Start the bot
sudo systemctl start polymarket-bot

# Check status
sudo systemctl status polymarket-bot

# View logs
sudo journalctl -u polymarket-bot -f
```

#### 8. Manage the Service

```bash
# Stop the bot
sudo systemctl stop polymarket-bot

# Restart the bot
sudo systemctl restart polymarket-bot

# View recent logs
sudo journalctl -u polymarket-bot -n 100

# Follow logs in real-time
sudo journalctl -u polymarket-bot -f
```

## Updating the Bot

### On Laptop
```bash
cd polymarket-arb-bot
# Make your changes
cargo build --release
```

### On VPS
```bash
# Stop the service
sudo systemctl stop polymarket-bot

# Pull updates (if using git)
cd ~/polymarket-arb-bot
git pull

# Or upload new version
# scp from laptop: scp -r polymarket-arb-bot user@vps-ip:~/

# Rebuild
cargo build --release

# Restart service
sudo systemctl start polymarket-bot
```

## Configuration Guide

### Core Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `DRY_RUN` | true | Set to `false` for live trading |
| `SIZE` | 5 | Fallback max shares per leg |
| `SUM_MAX` | 0.97 | Max price sum for arbitrage (0.97 = 3% edge) |
| `STALE_SECONDS` | 2.0 | Ignore book data older than this |
| `COOLDOWN_SECONDS` | 1.0 | Min time between arb attempts per symbol |

### Bankroll Management

| Parameter | Default | Description |
|-----------|---------|-------------|
| `BANKROLL_FRACTION` | 0.25 | Use 25% of available balance per trade |
| `RESERVE_USDC` | 2.0 | Keep $2 in reserve (not used for trading) |
| `MIN_LEG_NOTIONAL_USD` | 1.0 | Min $ value per leg (prevents tiny orders) |
| `MIN_TRADE_NOTIONAL_USD` | 2.0 | Min total trade size |
| `MAX_TRADE_NOTIONAL_USD` | 50.0 | Max total trade size (risk cap) |
| `SIZE_DECIMALS` | 2 | Round shares down to 2 decimal places |

### Mismatch Handling

When one leg fills but the other doesn't:

| Parameter | Default | Description |
|-----------|---------|-------------|
| `HEDGE_MODE` | refill_then_unwind | Strategy: `refill`, `unwind`, or `refill_then_unwind` |
| `HEDGE_SUM_MAX` | 0.98 | Max price sum when refilling missing leg |
| `HEDGE_RETRIES` | 2 | Attempts to refill missing leg |
| `UNWIND_RETRIES` | 3 | Attempts to unwind filled leg if refill fails |
| `HEDGE_SLEEP_MS` | 150 | Delay between refill attempts (ms) |
| `UNWIND_SLEEP_MS` | 150 | Delay between unwind attempts (ms) |

## Troubleshooting

### Build Errors

**Error:** `linker 'cc' not found`
```bash
# Ubuntu/Debian
sudo apt install build-essential

# macOS
xcode-select --install
```

**Error:** `failed to run custom build command for openssl-sys`
```bash
# Ubuntu/Debian
sudo apt install pkg-config libssl-dev

# macOS
brew install openssl
```

### Runtime Issues

**WebSocket won't connect**
- Check firewall settings
- Verify WSS_URL in .env
- Try: `curl -I https://clob.polymarket.com`

**No arbitrage opportunities**
- Market conditions may not favor arb
- Try lowering `SUM_MAX` (but increases risk)
- Verify markets are active (check Polymarket website)

**Orders rejected**
- Ensure token allowances are set
- Check USDC balance
- Verify private key is correct
- Check `SIGNATURE_TYPE` matches your wallet type

### Viewing Logs on VPS

```bash
# Last 100 lines
sudo journalctl -u polymarket-bot -n 100

# Follow live
sudo journalctl -u polymarket-bot -f

# Search for errors
sudo journalctl -u polymarket-bot | grep -i error

# Logs from last hour
sudo journalctl -u polymarket-bot --since "1 hour ago"
```

## Security Best Practices

1. **Private Key Security**
   - Never commit .env to git
   - Use environment variables on VPS
   - Consider using a hardware wallet or MPC solution

2. **VPS Security**
   - Set up SSH key authentication
   - Disable password login
   - Enable firewall (ufw)
   - Keep system updated

3. **Risk Management**
   - Start with small `SIZE` values
   - Use `DRY_RUN=true` extensively
   - Set reasonable `MAX_TRADE_NOTIONAL_USD`
   - Monitor regularly

## Performance vs Python

| Metric | Python | Rust |
|--------|--------|------|
| **Startup Time** | ~2s | ~50ms |
| **Memory Usage** | ~150MB | ~15MB |
| **Latency (order submission)** | ~50-100ms | ~5-15ms |
| **CPU Usage** | Higher | Lower |
| **Binary Size** | N/A (interpreted) | ~10MB (optimized) |

## Support & Resources

- **Polymarket Docs:** https://docs.polymarket.com
- **rs-clob-client:** https://github.com/Polymarket/rs-clob-client
- **Rust Book:** https://doc.rust-lang.org/book/

## Disclaimer

**This software is for educational purposes only.**

- Trading involves risk of loss
- No guarantees of profit
- Not financial advice
- Use at your own risk
- Test thoroughly before live trading

## License

MIT License - See LICENSE file